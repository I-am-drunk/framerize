use super::shared_lease::RunningAccountLease;
use crate::transport::downstream as downstream_owner;
use crate::account::client::AccountScope;
use crate::account::client::NativeAccountClient;
use crate::account::owner as native_owner;
use crate::request::types::AttemptMetadata;
use crate::request::types::Cancellation;
use crate::request::types::DispatchError;
use crate::request::types::FailureKind;
use crate::request::types::InvocationRun;
use futures::future::BoxFuture;
use futures::FutureExt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Context;
use std::task::Poll;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::Instant;

pub(super) struct Registry {
    // Raw starts once; each validated chain starts at most 64 attempts.
    preparations: Mutex<Vec<(crate::account::client::CloseObserver,
        crate::account::client::PreparationControl)>>,
    attempts: Mutex<Vec<(downstream_owner::Cleanup, native_owner::Cancellation)>>,
    completed: Mutex<Vec<AttemptMetadata>>,
    account_cleanup_required: AtomicBool,
}

impl Registry {
    pub(super) fn preparing(&self,
        cleanup: crate::account::client::CloseObserver,
        cancellation: crate::account::client::PreparationControl) {
        self.preparations.lock().unwrap_or_else(|e| e.into_inner()).push((cleanup, cancellation));
    }

    pub(super) fn require_account_cleanup(&self) {
        self.account_cleanup_required.store(true, Ordering::Release);
    }

    pub(super) fn completed(&self, metadata: AttemptMetadata) {
        self.completed.lock().unwrap_or_else(|e| e.into_inner()).push(metadata);
    }

    fn attach_metadata(&self, error: &mut DispatchError) {
        if error.upstream.is_none() && error.attempts.is_empty() {
            error.attempts = self.completed.lock().unwrap_or_else(|e| e.into_inner()).clone();
        }
    }
    pub(super) fn started(&self, cleanup: downstream_owner::Cleanup,
        cancellation: native_owner::Cancellation) {
        self.attempts.lock().unwrap_or_else(|e| e.into_inner()).push((cleanup, cancellation));
    }

    fn cancel(&self) {
        for (_, cancel) in &*self.preparations.lock().unwrap_or_else(|e| e.into_inner()) {
            cancel.cancel();
        }
        for (_, cancel) in &*self.attempts.lock().unwrap_or_else(|e| e.into_inner()) {
            cancel.cancel();
        }
    }
}

struct Signal(watch::Sender<bool>);
impl Cancellation for Signal { fn cancel(&self) { self.0.send_replace(true); } }
impl Signal {
    async fn cancelled(&self) {
        let mut rx = self.0.subscribe();
        loop {
            if *rx.borrow_and_update() { return; }
            if rx.changed().await.is_err() { return; }
        }
    }
}

#[derive(Clone, Copy)]
enum WorkerExit { Returned(bool), Lost }

struct Finished(Option<oneshot::Sender<WorkerExit>>);
impl Finished {
    fn finish(&mut self, success: bool) {
        if let Some(tx) = self.0.take() { let _ = tx.send(WorkerExit::Returned(success)); }
    }
}
impl Drop for Finished {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() { let _ = tx.send(WorkerExit::Lost); }
    }
}

struct ResultOwner<T> {
    rx: oneshot::Receiver<Result<T, DispatchError>>,
    signal: Arc<Signal>,
    done: bool,
}
impl<T> Unpin for ResultOwner<T> {}
impl<T> Future for ResultOwner<T> {
    type Output = Result<T, DispatchError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.rx).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(value) => {
                self.done = true;
                Poll::Ready(value.unwrap_or_else(|_| Err(DispatchError::new(FailureKind::Internal))))
            }
        }
    }
}
impl<T> Drop for ResultOwner<T> {
    fn drop(&mut self) { if !self.done { self.signal.cancel(); } }
}

pub(super) fn launch_with_lease<T, F, Fut>(native: NativeAccountClient, scope: AccountScope,
    permit: OwnedSemaphorePermit, account_lease: RunningAccountLease,
    deadline: Instant, max_attempts: usize, operation: F) -> InvocationRun<T>
where
    T: Send + 'static,
    F: FnOnce(Arc<Registry>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, DispatchError>> + Send + 'static,
{
    let (cancel_tx, _) = watch::channel(false);
    let signal = Arc::new(Signal(cancel_tx));
    let registry = Arc::new(Registry {
        preparations: Mutex::new(Vec::with_capacity(max_attempts)),
        attempts: Mutex::new(Vec::with_capacity(max_attempts)),
        completed: Mutex::new(Vec::with_capacity(max_attempts)),
        account_cleanup_required: AtomicBool::new(false),
    });
    let (finished_tx, finished_rx) = oneshot::channel();
    let (cleanup_tx, cleanup_rx) = oneshot::channel();
    let tracked = registry.clone();
    // This task owns capacity independently of both public future and worker.
    let guardian = tokio::spawn(async move {
        let outcome = finished_rx.await.unwrap_or(WorkerExit::Lost);
        let success = matches!(outcome, WorkerExit::Returned(true));
        let worker_lost = matches!(outcome, WorkerExit::Lost);
        let attempts = tracked.attempts.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if !success { for (_, cancel) in &attempts { cancel.cancel(); } }
        // An unwound worker may have started work before its observer ran.
        // An empty registry in that case is not proof that capacity is free.
        let mut result = if worker_lost { Err(FailureKind::Cleanup) } else { Ok(()) };
        let preparations = tracked.preparations.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if !success { for (_, cancel) in &preparations { cancel.cancel(); } }
        for (cleanup, _) in preparations {
            if cleanup.await.is_err() { result = Err(FailureKind::Cleanup); }
        }
        for (cleanup, _) in attempts {
            if cleanup.await.is_err() { result = Err(FailureKind::Cleanup); }
        }
        if !worker_lost && tracked.account_cleanup_required.load(Ordering::Acquire) {
            // A returned start error can leave work in the account supervisor
            // without returning Pending. Only a successful close proves its end.
            if scope.closed().await.is_err() { result = Err(FailureKind::Cleanup); }
        }
        let result = if result.is_ok() { account_lease.complete(&scope) } else {
            // Record poison before parking capacity after unconfirmed cleanup.
            drop(account_lease);
            result
        };
        if result.is_err() {
            let _ = cleanup_tx.send(result);
            // Failed cleanup is not proof that capacity is free. Quarantine
            // this reservation until runtime shutdown, without blocking the
            // typed failure delivered to its caller.
            futures::future::pending::<()>().await;
            drop((permit, scope, native));
        } else {
            drop((permit, scope, native));
            let _ = cleanup_tx.send(Ok(()));
        }
    });
    let cleanup = async move {
        match cleanup_rx.await {
            Ok(Ok(())) => guardian.await.map_err(|_| FailureKind::Cleanup),
            Ok(Err(kind)) => { drop(guardian); Err(kind) }
            Err(_) => { drop(guardian); Err(FailureKind::Cleanup) }
        }
    }.boxed().shared();
    let finished = Finished(Some(finished_tx));
    let completion = cleanup.clone();
    let worker_signal = signal.clone();
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let mut finished = finished;
        let mut result = tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => Err(DispatchError::new(FailureKind::Deadline)),
            _ = worker_signal.cancelled() => Err(DispatchError::new(FailureKind::Cancelled)),
            value = operation(registry.clone()) => value,
        };
        finished.finish(result.is_ok());
        if result.is_ok() {
            let completed = tokio::select! {
                biased;
                _ = tokio::time::sleep_until(deadline) => Err(FailureKind::Deadline),
                _ = worker_signal.cancelled() => Err(FailureKind::Cancelled),
                value = completion => value,
            };
            if let Err(kind) = completed {
                registry.cancel();
                result = Err(DispatchError::new(kind));
            }
        }
        if let Err(error) = &mut result { registry.attach_metadata(error); }
        let _ = tx.send(result);
    });
    let result: BoxFuture<'static, _> = Box::pin(ResultOwner {
        rx, signal: signal.clone(), done: false,
    });
    InvocationRun { result, cleanup, cancellation: signal }
}
