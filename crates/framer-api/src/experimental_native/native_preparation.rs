//! Account-owned work before native transport admission.
//! Cancellation suppresses publication; cleanup waits for the actual worker.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparationFailure {
    Rejected, Unavailable, Cancelled, Deadline, CleanupUnconfirmed, WorkerLost,
}

pub struct PreparationResult<T> {
    pub value: Result<T, PreparationFailure>,
    pub cleanup_confirmed: bool,
}
impl<T> PreparationResult<T> {
    pub fn finished(value: Result<T, PreparationFailure>) -> Self {
        Self { value, cleanup_confirmed: true }
    }
    pub fn unconfirmed() -> Self {
        Self { value: Err(PreparationFailure::CleanupUnconfirmed), cleanup_confirmed: false }
    }
}

#[derive(Clone)]
pub struct PreparationControl {
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}
impl PreparationControl {
    pub fn cancel(&self) { self.cancelled.store(true, Ordering::Release); }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline
    }
    pub fn deadline(&self) -> Instant { self.deadline }
}

pub struct PendingPreparation<T> {
    result: oneshot::Receiver<Result<T, PreparationFailure>>,
    settled: CloseObserver,
    control: PreparationControl,
}
impl<T> PendingPreparation<T> {
    pub fn completion(&self) -> CloseObserver { self.settled.clone() }
    pub fn cancellation(&self) -> PreparationControl { self.control.clone() }
}
impl<T> Unpin for PendingPreparation<T> {}
impl<T> Drop for PendingPreparation<T> {
    fn drop(&mut self) { self.control.cancel(); }
}
impl<T> Future for PendingPreparation<T> {
    type Output = Result<T, PreparationFailure>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match Pin::new(&mut this.settled).poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(_)) => return Poll::Ready(Err(PreparationFailure::CleanupUnconfirmed)),
            Poll::Ready(Ok(())) => {}
        }
        if this.control.is_cancelled() {
            return Poll::Ready(Err(PreparationFailure::Cancelled));
        }
        Pin::new(&mut this.result).poll(cx)
            .map(|result| result.unwrap_or(Err(PreparationFailure::WorkerLost)))
    }
}

pub(super) struct Work {
    pub(super) control: PreparationControl,
    driver: Option<CloseObserver>,
    pub(super) settled: Option<oneshot::Sender<CloseResult>>,
}

impl NativeAccountClient {
    #[cfg(test)]
    pub(crate) fn prepare_at_registration_for_test<T, F, Fut>(
        &self, scope: &AccountScope, deadline: Instant, work: F, hook: impl FnOnce(),
    ) -> Result<PendingPreparation<T>, ClientError>
    where T: Send + 'static,
        F: FnOnce(Session, PreparationControl) -> Fut + Send + 'static,
        Fut: Future<Output = PreparationResult<T>> + Send + 'static,
    {
        self.prepare_with(scope, deadline, work, hook)
    }

    /// Reserve before invoking work, using this generation's stored session.
    /// The future must own and observe all external work it starts. A dropped
    /// waiter cannot abort it. Unknown cleanup closes the generation.
    pub fn prepare_owned<T, F, Fut>(
        &self, scope: &AccountScope, deadline: Instant, work: F,
    ) -> Result<PendingPreparation<T>, ClientError>
    where T: Send + 'static,
        F: FnOnce(Session, PreparationControl) -> Fut + Send + 'static,
        Fut: Future<Output = PreparationResult<T>> + Send + 'static,
    {
        self.prepare_with(scope, deadline, work, || {})
    }

    fn prepare_with<T, F, Fut>(&self, scope: &AccountScope, deadline: Instant,
        work: F, before_registration: impl FnOnce()) -> Result<PendingPreparation<T>, ClientError>
    where T: Send + 'static,
        F: FnOnce(Session, PreparationControl) -> Fut + Send + 'static,
        Fut: Future<Output = PreparationResult<T>> + Send + 'static,
    {
        if !Weak::ptr_eq(&scope.owner, &Arc::downgrade(&self.inner)) {
            return Err(ClientError::WrongClient);
        }
        if deadline <= Instant::now() { return Err(ClientError::InvalidContext); }
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| ClientError::NoRuntime)?;
        let (operation, session) = {
            let mut state = lock(&self.inner.state);
            if !state.healthy { return Err(ClientError::Unavailable); }
            let index = state.accounts.iter()
                .position(|a| a.identity.generation == scope.generation())
                .ok_or(ClientError::StaleScope)?;
            if state.accounts[index].closing { return Err(ClientError::Draining); }
            if state.accounts.iter().map(|a| a.requests.len()).sum::<usize>() >= CAPACITY {
                return Err(ClientError::Full);
            }
            let operation = sequence(&mut state)?;
            let account = &mut state.accounts[index];
            let session = account.resources.as_ref().ok_or(ClientError::Draining)?.session.clone();
            account.requests.push(Operation { id: operation, stage: OperationStage::Preparing });
            (operation, session)
        };
        let mut reservation = Reservation {
            inner: self.inner.clone(), generation: scope.generation(),
            operation: Some(operation), armed: true, may_have_started: false,
        };
        let control = PreparationControl {
            cancelled: Arc::new(AtomicBool::new(false)), deadline,
        };
        let worker_control = control.clone();
        let (tx, result) = oneshot::channel();
        let (settled_tx, settled) = signal();
        reservation.may_have_started = true;
        let worker = runtime.spawn(async move {
            if worker_control.is_cancelled() {
                return PreparationResult::finished(Err(PreparationFailure::Cancelled));
            }
            work(session, worker_control).await
        });
        let mut worker = OwnedWorker(worker);
        let driver = async move {
            match (&mut worker.0).await {
                Ok(outcome) => {
                    let cleanup = if outcome.cleanup_confirmed { Ok(()) }
                        else { Err(CloseError::PreparationUnconfirmed) };
                    let _ = tx.send(outcome.value);
                    cleanup
                }
                Err(_) => {
                    let _ = tx.send(Err(PreparationFailure::WorkerLost));
                    Err(CloseError::PreparationUnconfirmed)
                }
            }
        }.boxed().shared();
        before_registration();
        let active = self.register(&mut reservation, OperationStage::OwnedPreparation(Work {
            control: control.clone(), driver: Some(CloseObserver(driver)),
            settled: Some(settled_tx),
        }), None);
        if !active {
            control.cancel();
        }
        // Work already exists. Even a concurrent close must return its real
        // completion so invocation owners retain capacity until it settles.
        Ok(PendingPreparation { result, settled, control })
    }
}

// A lost supervisor must not detach an unobserved worker. Its account barrier
// stays failed: aborting is cancellation, never evidence of external cleanup.
struct OwnedWorker<T>(JoinHandle<T>);
impl<T> Drop for OwnedWorker<T> {
    fn drop(&mut self) { self.0.abort(); }
}

#[cfg(test)]
mod registration_tests {
    use super::*;

    #[tokio::test]
    async fn supervisor_loss_before_preparation_registration_settles_failure() {
        let client = NativeAccountClient::new().unwrap();
        let scope = client.open_account(AccountId(uuid::Uuid::new_v4()), Session {
            active_user: FramerUserId("fixture-user".to_owned()),
            cookie_header: "session=fixture-r3".to_owned(),
        }, NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
        let pending = client.prepare_at_registration_for_test(&scope,
            Instant::now() + std::time::Duration::from_secs(30),
            |_, _| async { PreparationResult::finished(Ok(())) },
            || {
                client.supervisor_for_test().abort();
                client.inner.supervisor_lost();
            }).unwrap();
        // No supervisor is left to resolve this newly registered observer.
        assert_eq!(pending.completion().now_or_never(),
            Some(Err(CloseError::SupervisorLost)));
        assert!(pending.cancellation().is_cancelled());
        assert_eq!(pending.await, Err(PreparationFailure::CleanupUnconfirmed));
        assert_eq!(scope.closed().await, Err(CloseError::SupervisorLost));
        let state = lock(&client.inner.state);
        let account = state.accounts.iter().find(|a|
            a.identity.generation == scope.generation()).unwrap();
        assert!(account.closing);
        assert_eq!(account.requests.len(), 1);
        assert!(matches!(account.requests[0].stage, OperationStage::OwnedPreparation(_)));
    }
}

impl Inner {
    pub(super) fn poll_preparations(&self, cx: &mut Context<'_>) {
        let snapshot: Vec<_> = {
            let mut state = lock(&self.state);
            state.accounts.iter_mut().flat_map(|account| {
                let generation = account.identity.generation;
                account.requests.iter_mut().filter_map(move |request| {
                    if let OperationStage::OwnedPreparation(work) = &mut request.stage {
                        work.driver.take().map(|driver| (generation, request.id, driver))
                    } else { None }
                })
            }).collect()
        };
        let mut acknowledgements = Vec::new();
        for (generation, operation, mut driver) in snapshot {
            let outcome = Pin::new(&mut driver).poll(cx);
            let mut state = lock(&self.state);
            let account = state.accounts.iter_mut()
                .find(|a| a.identity.generation == generation)
                .expect("owned preparation retains its account");
            let index = account.requests.iter().position(|r| r.id == operation)
                .expect("owned preparation retains its slot");
            match outcome {
                Poll::Pending => {
                    let OperationStage::OwnedPreparation(work) = &mut account.requests[index].stage
                        else { unreachable!() };
                    work.driver = Some(driver);
                }
                Poll::Ready(result) => {
                    let stage = if result.is_ok() {
                        account.requests.remove(index).stage
                    } else {
                        // Unknown external cleanup must not release the
                        // shared preparation/request reservation.
                        std::mem::replace(&mut account.requests[index].stage,
                            OperationStage::Unconfirmed)
                    };
                    let OperationStage::OwnedPreparation(mut work) = stage
                        else { unreachable!() };
                    if let Err(error) = result {
                        account.failure.get_or_insert(error);
                        account.closing = true;
                    }
                    if let Some(sender) = work.settled.take() {
                        acknowledgements.push((sender, result));
                    }
                }
            }
        }
        let controls: Vec<_> = {
            let state = lock(&self.state);
            state.accounts.iter().filter(|a| a.closing).flat_map(cancellations).collect()
        };
        for control in controls { control.cancel(); }
        // The slot and worker are gone before publication enables native start.
        for (sender, result) in acknowledgements { let _ = sender.send(result); }
    }
}
