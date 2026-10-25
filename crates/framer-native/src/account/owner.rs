//! Source-only owner for the accepted native transport and response adapter.
//!
//! Share one RequestPool across accounts. Its limit is process-local; creating
//! another pool creates another budget. No retries, token minting, body pump,
//! response history, or credential-bearing Debug implementation lives here.

use crate::transport::http::AccountRequestAuth;

use crate::transport::http::FreshCastleToken;

use crate::transport::http::NativeCancellation;

use crate::transport::http::NativeCloseError;

use crate::transport::http::NativeHttp;
use crate::transport::response::adapt_response;
use crate::transport::response::AdaptedResponse;
use crate::transport::response::AdapterError;
use crate::transport::response::Mode;
use framer_core::ids::AccountId;
use framer_core::port::Session;
use futures::task::waker_ref;
use futures::task::ArcWake;
use futures::task::AtomicWaker;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::task::Context;
use std::task::Poll;
use tokio::task::JoinHandle;
use tokio::time::Instant;

pub const MAX_REQUESTS: usize = 10;

/// Owned, immutable, consumed-once request material. The caller supplies the
/// genuine opaque token and its provenance. The native account, session,
/// freshness, route, credential and envelope checks remain authoritative.
pub struct Inputs {
    account: AccountId,
    session: Session,
    bearer: String,
    path: String,
    body: Value,
    token: FreshCastleToken,
    client: Arc<NativeHttp>,
}

impl Inputs {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        account: AccountId,
        session: Session,
        bearer: String,
        path: String,
        body: Value,
        token: FreshCastleToken,
        client: Arc<NativeHttp>,
    ) -> Self {
        Self { account, session, bearer, path, body, token, client }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    InvalidLimit,
    Full,
    NoRuntime,
    Shutdown,
    SpawnFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Deadline,
    Cancelled,
    OwnerDropped,
    ReceiverDropped,
    PoolDropped,
}

pub enum ResponseError {
    Stopped(StopReason),
    Adapter(AdapterError),
    TaskLost,
    AlreadyTaken,
}

impl std::fmt::Debug for ResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped(reason) => f.debug_tuple("Stopped").field(reason).finish(),
            Self::Adapter(_) => f.write_str("native-owner: adapter failed"),
            Self::TaskLost => f.write_str("native-owner: driver lost"),
            Self::AlreadyTaken => f.write_str("native-owner: response already taken"),
        }
    }
}

pub type ResponseResult = Result<AdaptedResponse, ResponseError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    Response,
    Stopped(StopReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionError {
    Native(NativeCloseError),
    TaskLost,
}

/// Ok means BOTH the original NativeClosed succeeded and the JoinHandle
/// returned normally. It does not assert HTTP or terminal semantic success.
pub type CompletionResult = Result<EndReason, CompletionError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolSnapshot {
    pub limit: usize,
    pub occupied: usize,
    pub quarantined: usize,
}

#[derive(Clone)]
pub struct RequestPool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    limit: usize,
    shutting_down: AtomicBool,
    slots: Mutex<[Slot; MAX_REQUESTS]>,
}

enum Slot {
    Vacant,
    Reserved(Arc<Shared>),
    Running { shared: Arc<Shared>, handle: JoinHandle<DriverExit> },
    Reaping(Arc<Shared>),
    Quarantined(Arc<Shared>),
}

struct Shared {
    deadline: Instant,
    cancellation: NativeCancellation,
    state: Mutex<State>,
    driver_waker: AtomicWaker,
    response_waker: AtomicWaker,
    completion_waker: AtomicWaker,
    join_notified: AtomicBool,
    #[cfg(test)]
    abort: Mutex<Option<tokio::task::AbortHandle>>,
}

struct State {
    /// This is the only response storage. Taking or replacing it moves the
    /// previous value out of the mutex before any destructor is run.
    result: Option<ResponseResult>,
    delivery: Delivery,
    stop: Option<StopReason>,
    driver_done: bool,
    completion: Option<CompletionResult>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Delivery { Waiting, Queued, Taken, Abandoned }

struct DriverExit {
    reason: EndReason,
    native: Result<(), NativeCloseError>,
}

/// The sole response receiver. Dropping it before receipt requests cleanup.
pub struct ResponseReceiver {
    pool: RequestPool,
    shared: Arc<Shared>,
    done: bool,
}

/// The sole completion receiver. Polling reaps the actual Tokio JoinHandle.
/// Keeping it after dropping RequestOwner permits observing that cleanup.
pub struct Completion {
    pool: RequestPool,
    shared: Arc<Shared>,
}

#[derive(Clone)]
pub struct Cancellation {
    shared: Arc<Shared>,
}

pub struct RequestOwner {
    _pool: RequestPool,
    shared: Arc<Shared>,
    response: Option<ResponseReceiver>,
    completion: Option<Completion>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Shared {
    fn new(deadline: Instant, cancellation: NativeCancellation) -> Self {
        Self {
            deadline,
            cancellation,
            state: Mutex::new(State {
                result: None, delivery: Delivery::Waiting, stop: None,
                driver_done: false, completion: None,
            }),
            driver_waker: AtomicWaker::new(),
            response_waker: AtomicWaker::new(),
            completion_waker: AtomicWaker::new(),
            join_notified: AtomicBool::new(false),
            #[cfg(test)]
            abort: Mutex::new(None),
        }
    }
}

// Every JoinHandle is polled with this stable proxy waker, never with another
// request's observer. Reaping cannot steal another completion's notification.
impl ArcWake for Shared {
    fn wake_by_ref(shared: &Arc<Self>) {
        shared.join_notified.store(true, Ordering::Release);
        shared.completion_waker.wake();
        shared.response_waker.wake();
    }
}

impl Shared {
    fn stopped(&self) -> Option<StopReason> { lock(&self.state).stop }

    fn stop_if_due(&self) {
        if Instant::now() >= self.deadline { self.request_stop(StopReason::Deadline); }
    }

    fn request_stop(&self, requested: StopReason) {
        let discarded = {
            let mut state = lock(&self.state);
            if state.driver_done || state.completion.is_some() || state.stop.is_some() { return; }
            let reason = if Instant::now() >= self.deadline {
                StopReason::Deadline
            } else {
                requested
            };
            state.stop = Some(reason);
            match state.delivery {
                Delivery::Waiting | Delivery::Queued => {
                    state.delivery = Delivery::Queued;
                    state.result.replace(Err(ResponseError::Stopped(reason)))
                }
                Delivery::Taken | Delivery::Abandoned => None,
            }
        };
        // Native cancellation can dispose a body and wake arbitrary observers.
        // Neither it nor destruction of a queued response runs under our lock.
        self.cancellation.cancel();
        drop(discarded);
        self.driver_waker.wake();
        self.response_waker.wake();
    }

    fn publish(&self, result: ResponseResult) {
        self.stop_if_due();
        let mut result = Some(result);
        {
            let mut state = lock(&self.state);
            if state.delivery == Delivery::Waiting && state.stop.is_none() {
                state.result = result.take();
                state.delivery = Delivery::Queued;
            }
        }
        drop(result);
        self.response_waker.wake();
    }

    fn abandon(&self) {
        let discarded = {
            let mut state = lock(&self.state);
            if state.delivery == Delivery::Taken || state.delivery == Delivery::Abandoned {
                return;
            }
            state.delivery = Delivery::Abandoned;
            state.result.take()
        };
        self.request_stop(StopReason::ReceiverDropped);
        drop(discarded);
    }

    fn driver_finished(&self) -> EndReason {
        let reason = {
            let mut state = lock(&self.state);
            state.driver_done = true;
            state.stop.map_or(EndReason::Response, EndReason::Stopped)
        };
        drop(self.driver_waker.take());
        reason
    }

    fn joined(&self, completion: CompletionResult) {
        let discarded = {
            let mut state = lock(&self.state);
            state.driver_done = true;
            state.completion = Some(completion);
            if matches!(completion, Err(CompletionError::TaskLost))
                && matches!(state.delivery, Delivery::Waiting | Delivery::Queued)
            {
                state.delivery = Delivery::Queued;
                state.result.replace(Err(ResponseError::TaskLost))
            } else { None }
        };
        if completion.is_err() { self.cancellation.cancel(); }
        drop(discarded);
        drop(self.driver_waker.take());
        self.response_waker.wake();
        self.completion_waker.wake();
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct TestHooks {
    pub tail_gate: Option<futures::channel::oneshot::Receiver<()>>,
    pub lose_native_closed: bool,
}

#[cfg(test)]
type DriverHooks = TestHooks;

#[cfg(not(test))]
#[derive(Default)]
struct DriverHooks;

// A lost driver must cancel an already published body even when nobody
// polls a result/JoinHandle again. This guard makes no cleanup success claim.
struct CancelOnDrop(Option<NativeCancellation>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancellation) = &self.0 { cancellation.cancel(); }
    }
}

/// Check stop/deadline before polling work and after it becomes ready.
/// A latched stop wins; otherwise an expired deadline wins over a cancel
/// or response first observed in the same poll.
async fn controlled<F: Future>(
    shared: &Shared,
    work: F,
) -> Result<F::Output, StopReason> {
    let mut work = Box::pin(work);
    let mut deadline = Box::pin(tokio::time::sleep_until(shared.deadline));
    let result = futures::future::poll_fn(|cx| {
        shared.driver_waker.register(cx.waker());
        let expired = deadline.as_mut().poll(cx).is_ready();
        if expired { shared.request_stop(StopReason::Deadline); }
        shared.stop_if_due();
        if let Some(reason) = shared.stopped() {
            return Poll::Ready(Err(reason));
        }
        match work.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(value) => {
                shared.stop_if_due();
                if let Some(reason) = shared.stopped() {
                    drop(value);
                    Poll::Ready(Err(reason))
                } else {
                    Poll::Ready(Ok(value))
                }
            }
        }
    }).await;
    // This must precede awaiting NativeClosed, including when stopped.
    drop(work);
    result
}

async fn drive(
    inputs: Inputs,
    mode: Mode,
    shared: Arc<Shared>,
    #[allow(unused_variables)] hooks: DriverHooks,
) -> DriverExit {
    let mut cleanup = CancelOnDrop(Some(shared.cancellation.clone()));
    let Inputs { account, session, bearer, path, body, token, client } = inputs;
    let (head, mut closed) = {
        let request = client.ai_post_stream_tracked(
            &account,
            &path,
            AccountRequestAuth::new(account, &session, &bearer),
            &body,
            token,
            shared.cancellation.clone(),
        );
        // Construction alone never polls/sends. Even a stop before the
        // first task poll drops a real tracked send and keeps its close ack.
        let head = controlled(&shared, request.response).await;
        (head, request.closed)
    };
    // Release request JSON, credentials and the client copy after headers.
    // No request borrows enter the independently owned response phase.
    drop((session, bearer, path, body, client));
    match head {
        Ok(Ok(response)) => {
            if let Ok(result) = controlled(&shared, adapt_response(response, mode)).await {
                shared.publish(result.map_err(ResponseError::Adapter));
            }
        }
        Ok(Err(error)) => {
            shared.publish(Err(ResponseError::Adapter(AdapterError::Send(error))));
        }
        Err(_) => {}
    }

    // Raw/rejected responses may be queued or claimed but still unpolled.
    // Keep the absolute deadline over that same original native lifetime.
    let native = match controlled(&shared, &mut closed).await {
        Ok(result) => result,
        Err(_) => {
            shared.cancellation.cancel();
            closed.await
        }
    };
    if native.is_err() { shared.cancellation.cancel(); }
    cleanup.0 = None;

    #[cfg(test)]
    let native = if hooks.lose_native_closed {
        // Observe real cleanup first, then exercise the lost-ack path.
        crate::transport::http::closure::lost_signal_for_test().await
    } else { native };

    let reason = shared.driver_finished();
    #[cfg(test)]
    if let Some(gate) = hooks.tail_gate { let _ = gate.await; }
    DriverExit { reason, native }
}

impl RequestOwner {
    pub fn take_response(&mut self) -> Option<ResponseReceiver> { self.response.take() }

    pub fn take_completion(&mut self) -> Option<Completion> { self.completion.take() }

    pub fn cancellation(&self) -> Cancellation {
        Cancellation { shared: Arc::clone(&self.shared) }
    }

    pub fn cancel(&self) { self.shared.request_stop(StopReason::Cancelled); }

    #[cfg(test)]
    pub fn abort_task_for_test(&self) -> bool {
        let abort = lock(&self.shared.abort).clone();
        match abort {
            Some(abort) => { abort.abort(); true }
            None => false,
        }
    }
}

impl Drop for RequestOwner {
    fn drop(&mut self) { self.shared.request_stop(StopReason::OwnerDropped); }
}

impl Cancellation {
    pub fn cancel(&self) { self.shared.request_stop(StopReason::Cancelled); }
}

impl Future for ResponseReceiver {
    type Output = ResponseResult;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.done { return Poll::Ready(Err(ResponseError::AlreadyTaken)); }
        this.shared.response_waker.register(cx.waker());
        this.shared.stop_if_due();
        this.pool.reap();
        let result = {
            let mut state = lock(&this.shared.state);
            let result = state.result.take();
            if result.is_some() { state.delivery = Delivery::Taken; }
            result
        };
        match result {
            Some(result) => {
                this.done = true;
                drop(this.shared.response_waker.take());
                Poll::Ready(result)
            }
            None => Poll::Pending,
        }
    }
}

impl Drop for ResponseReceiver {
    fn drop(&mut self) {
        drop(self.shared.response_waker.take());
        if !self.done { self.shared.abandon(); }
    }
}

impl Future for Completion {
    type Output = CompletionResult;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.shared.completion_waker.register(cx.waker());
        this.shared.stop_if_due();
        this.pool.reap();
        let result = { lock(&this.shared.state).completion };
        match result {
            Some(result) => {
                drop(this.shared.completion_waker.take());
                Poll::Ready(result)
            }
            None => Poll::Pending,
        }
    }
}

impl Drop for Completion {
    fn drop(&mut self) { drop(self.shared.completion_waker.take()); }
}

impl RequestPool {
    pub fn new(limit: usize) -> Result<Self, AdmissionError> {
        if limit == 0 || limit > MAX_REQUESTS { return Err(AdmissionError::InvalidLimit); }
        Ok(Self { inner: Arc::new(PoolInner {
            limit,
            shutting_down: AtomicBool::new(false),
            slots: Mutex::new(std::array::from_fn(|_| Slot::Vacant)),
        }) })
    }

    pub fn start(
        &self, inputs: Inputs, mode: Mode, deadline: Instant,
    ) -> Result<RequestOwner, AdmissionError> {
        self.start_inner(inputs, mode, deadline, DriverHooks::default())
    }

    #[cfg(test)]
    pub fn start_for_test(
        &self, inputs: Inputs, mode: Mode, deadline: Instant, hooks: TestHooks,
    ) -> Result<RequestOwner, AdmissionError> {
        self.start_inner(inputs, mode, deadline, hooks)
    }

    fn start_inner(
        &self, inputs: Inputs, mode: Mode, deadline: Instant, hooks: DriverHooks,
    ) -> Result<RequestOwner, AdmissionError> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(AdmissionError::Shutdown);
        }
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| AdmissionError::NoRuntime)?;
        self.reap();
        let shared = Arc::new(Shared::new(deadline, NativeCancellation::new()));
        let index = {
            let mut slots = lock(&self.inner.slots);
            if self.inner.shutting_down.load(Ordering::Acquire) {
                return Err(AdmissionError::Shutdown);
            }
            let index = (0..self.inner.limit)
                .find(|&i| matches!(slots[i], Slot::Vacant))
                .ok_or(AdmissionError::Full)?;
            slots[index] = Slot::Reserved(Arc::clone(&shared));
            index
        };
        let future = drive(inputs, mode, Arc::clone(&shared), hooks);
        let spawned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || runtime.spawn(future)
        ));
        let handle = match spawned {
            Ok(handle) => handle,
            Err(payload) => {
                drop(payload);
                let old = {
                    let mut slots = lock(&self.inner.slots);
                    std::mem::replace(&mut slots[index], Slot::Quarantined(Arc::clone(&shared)))
                };
                drop(old);
                shared.joined(Err(CompletionError::TaskLost));
                return Err(AdmissionError::SpawnFailed);
            }
        };
        #[cfg(test)]
        {
            *lock(&shared.abort) = Some(handle.abort_handle());
        }
        let old = {
            let mut slots = lock(&self.inner.slots);
            std::mem::replace(&mut slots[index],
                Slot::Running { shared: Arc::clone(&shared), handle })
        };
        drop(old);
        let owner = RequestOwner {
            _pool: self.clone(),
            shared: Arc::clone(&shared),
            response: Some(ResponseReceiver {
                pool: self.clone(), shared: Arc::clone(&shared), done: false,
            }),
            completion: Some(Completion { pool: self.clone(), shared }),
        };
        // Also register join wakeups for task failures before any consumer poll.
        self.reap();
        Ok(owner)
    }

    /// Poll each retained JoinHandle once. Only a successful native cleanup
    /// and an actually completed task return the slot to the free list.
    /// Failed tasks/acknowledgements permanently quarantine their pool slot.
    pub fn reap(&self) -> usize {
        let mut joined = 0;
        for index in 0..self.inner.limit {
            let taken = {
                let mut slots = lock(&self.inner.slots);
                let shared = match &slots[index] {
                    Slot::Running { shared, .. } => Arc::clone(shared),
                    _ => continue,
                };
                std::mem::replace(&mut slots[index], Slot::Reaping(shared))
            };
            let Slot::Running { shared, mut handle } = taken else { unreachable!() };
            shared.join_notified.store(false, Ordering::Release);
            let result = {
                let waker = waker_ref(&shared);
                let mut context = Context::from_waker(&waker);
                Pin::new(&mut handle).poll(&mut context)
            };
            match result {
                Poll::Pending => {
                    let old = {
                        let mut slots = lock(&self.inner.slots);
                        std::mem::replace(&mut slots[index], Slot::Running {
                            shared: Arc::clone(&shared), handle,
                        })
                    };
                    drop(old);
                    // A wake may have synchronously reentered while this
                    // handle was checked out. Repeat it after restoration.
                    if shared.join_notified.swap(false, Ordering::AcqRel) {
                        shared.completion_waker.wake();
                        shared.response_waker.wake();
                    }
                }
                Poll::Ready(result) => {
                    let completion = match result {
                        Ok(exit) => exit.native.map(|()| exit.reason)
                            .map_err(CompletionError::Native),
                        Err(_) => Err(CompletionError::TaskLost),
                    };
                    let replacement = if completion.is_ok() {
                        Slot::Vacant
                    } else {
                        Slot::Quarantined(Arc::clone(&shared))
                    };
                    let old = {
                        let mut slots = lock(&self.inner.slots);
                        std::mem::replace(&mut slots[index], replacement)
                    };
                    drop(old);
                    drop(handle);
                    joined += 1;
                    shared.joined(completion);
                }
            }
        }
        joined
    }

    pub fn snapshot(&self) -> PoolSnapshot {
        self.reap();
        let slots = lock(&self.inner.slots);
        PoolSnapshot {
            limit: self.inner.limit,
            occupied: slots[..self.inner.limit].iter()
                .filter(|slot| !matches!(slot, Slot::Vacant)).count(),
            quarantined: slots[..self.inner.limit].iter()
                .filter(|slot| matches!(slot, Slot::Quarantined(_))).count(),
        }
    }

    /// Stop accepting new work and ask every admitted request to clean up.
    /// Existing observers still wait for real native closure and task joins.
    pub fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::Release);
        let requests: [Option<Arc<Shared>>; MAX_REQUESTS] = {
            let slots = lock(&self.inner.slots);
            std::array::from_fn(|index| match &slots[index] {
                Slot::Vacant => None,
                Slot::Reserved(shared) | Slot::Reaping(shared) | Slot::Quarantined(shared)
                    | Slot::Running { shared, .. } => Some(Arc::clone(shared)),
            })
        };
        for shared in requests.into_iter().flatten() {
            shared.request_stop(StopReason::PoolDropped);
        }
        self.reap();
    }
}

impl Drop for PoolInner {
    fn drop(&mut self) {
        self.shutting_down.store(true, Ordering::Release);
        let slots = std::mem::replace(
            self.slots.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner),
            std::array::from_fn(|_| Slot::Vacant),
        );
        // No RequestPool/owner/receiver remains to observe joins. Cancellation
        // closes claimed bodies immediately; abort makes pending task futures
        // get destroyed by their runtime instead of detaching active work.
        for slot in slots {
            match slot {
                Slot::Vacant => {}
                Slot::Running { shared, handle } => {
                    shared.request_stop(StopReason::PoolDropped);
                    handle.abort();
                    drop(handle);
                }
                Slot::Reserved(shared) | Slot::Reaping(shared) | Slot::Quarantined(shared) => {
                    shared.request_stop(StopReason::PoolDropped);
                }
            }
        }
    }
}
