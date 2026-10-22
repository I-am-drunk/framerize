//! Pull-driven raw delivery. The guardian owns admission until real cleanup
//! and every returned byte reservation complete. Normal startup stays dormant.
use crate::experimental_native::native_account::{CloseObserver, PreparationControl};
use super::{errors, NativeCall, Parts};
use crate::experimental_native::{application::{ApplicationError, AttemptAuthorization,
    AuthorizedAttempt, PreparedAttempt}, downstream_owner as bridge,
    native_account::ClientError, native_owner};
use crate::proxy::Dialect;
use bytes::Bytes;
use framer_core::{convert::ClientDialect, native_dispatch::{AttemptMetadata,
    Cancellation, DispatchError, FailureKind, InvocationRun, RawStreamCompletion,
    RawStreamHead, RawStreamHeaders, RawStreamLimits, RawStreamRun, Route}};
use futures::{future::BoxFuture, task::AtomicWaker, Future, FutureExt, Stream};
use serde_json::Value;
use std::{pin::Pin, sync::{Arc, Mutex, MutexGuard}, task::{Context, Poll}};
use tokio::sync::{oneshot, watch, AcquireError, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

type Source = Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>>;
type Acquire = BoxFuture<'static, Result<OwnedSemaphorePermit, AcquireError>>;

#[cfg(test)]
pub(super) type TestStart = Box<dyn FnOnce(&Parts, Value)
    -> Result<bridge::Pending, ApplicationError> + Send>;

#[derive(Default)]
struct Hooks { #[cfg(test)] start: Option<TestStart> }

struct Pull {
    source: Source,
    cleanup: bridge::Cleanup,
    credit: Option<OwnedSemaphorePermit>,
    acquiring: Option<Acquire>,
    drained: bool,
}

struct State {
    pull: Option<Pull>,
    headers: Option<Arc<RawStreamHeaders>>,
    end: Option<Result<(), FailureKind>>,
    emitted: usize,
    elapsed: std::time::Duration,
    // A failed account start can have admitted work without returning Pending.
    // Only the account supervisor can then confirm actual cleanup.
    require_scope_cleanup: bool,
}

struct Control {
    state: Mutex<State>,
    native: Mutex<Option<(bridge::Cleanup, native_owner::Cancellation)>>,
    preparation: Mutex<Option<(CloseObserver, PreparationControl)>>,
    stop: watch::Sender<Option<FailureKind>>,
    ended: watch::Sender<bool>,
    head_waker: AtomicWaker,
    body_waker: AtomicWaker,
    budget: Arc<Semaphore>,
    limits: RawStreamLimits,
    started: Instant,
    deadline: Instant,
    step_index: u32,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Control {
    fn finish(&self, mut result: Result<(), FailureKind>) {
        let discarded = {
            let mut state = lock(&self.state);
            if state.end.is_some() { return; }
            if Instant::now() >= self.deadline { result = Err(FailureKind::Deadline); }
            state.end = Some(result);
            state.elapsed = self.started.elapsed();
            state.pull.take()
        };
        if let Err(kind) = result {
            self.stop.send_replace(Some(kind));
            if let Some((_, cancel)) = &*lock(&self.preparation) { cancel.cancel(); }
            let cancel = lock(&self.native).as_ref().map(|(_, c)| c.clone());
            if let Some(cancel) = cancel { cancel.cancel(); }
        }
        drop(discarded);
        self.ended.send_replace(true);
        self.head_waker.wake();
        self.body_waker.wake();
    }

    fn check_deadline(&self) {
        if Instant::now() >= self.deadline { self.finish(Err(FailureKind::Deadline)); }
    }

    async fn stopped(&self) -> FailureKind {
        let mut receiver = self.stop.subscribe();
        loop {
            if let Some(kind) = *receiver.borrow_and_update() { return kind; }
            if receiver.changed().await.is_err() { return FailureKind::Internal; }
        }
    }

    async fn body_finished(&self) {
        let mut receiver = self.ended.subscribe();
        loop {
            if *receiver.borrow_and_update() { return; }
            if receiver.changed().await.is_err() { return; }
        }
    }

    fn summary(&self, cleanup_failed: bool) -> RawStreamCompletion {
        let state = lock(&self.state);
        let mut result = state.end.unwrap_or(Err(FailureKind::Internal));
        if cleanup_failed && result.is_ok() { result = Err(FailureKind::Cleanup); }
        RawStreamCompletion { result, headers: state.headers.clone(),
            emitted_bytes: state.emitted,
            elapsed: if state.end.is_some() { state.elapsed } else { self.started.elapsed() } }
    }

    fn error(&self, kind: FailureKind) -> DispatchError {
        let state = lock(&self.state);
        DispatchError { kind, upstream: None,
            attempts: state.headers.as_ref().map(|headers| vec![AttemptMetadata {
                status: headers.status,
                usage_header: headers.usage_header.as_ref().map(|v| v.to_string()),
                original_content_type: headers.original_content_type.as_ref().map(|v| v.to_string()),
                step_index: self.step_index, elapsed: if state.end.is_some() { state.elapsed } else { self.started.elapsed() },
            }]).unwrap_or_default() }
    }

    fn restore(&self, pull: Pull) {
        let discarded = {
            let mut state = lock(&self.state);
            if state.end.is_none() { state.pull.replace(pull) } else { Some(pull) }
        };
        drop(discarded);
    }
}

impl Cancellation for Control {
    fn cancel(&self) { self.finish(Err(FailureKind::Cancelled)); }
}

// Drop data before credit so another poll cannot reuse the budget while the
// previous allocation still exists. Bytes clones/slices retain this owner.
struct ChargedBytes { data: Box<[u8]>, _credit: OwnedSemaphorePermit }
impl AsRef<[u8]> for ChargedBytes { fn as_ref(&self) -> &[u8] { &self.data } }

struct Body { control: Arc<Control>, done: bool }

impl Body {
    fn ended(&mut self, result: Result<(), FailureKind>) -> Poll<Option<Result<Bytes, FailureKind>>> {
        self.done = true;
        match result { Ok(()) => Poll::Ready(None), Err(kind) => Poll::Ready(Some(Err(kind))) }
    }

    fn fail(&mut self, pull: Pull, kind: FailureKind) -> Poll<Option<Result<Bytes, FailureKind>>> {
        self.control.finish(Err(kind));
        drop(pull);
        let result = lock(&self.control.state).end.unwrap_or(Err(kind));
        self.ended(result)
    }
}

impl Stream for Body {
    type Item = Result<Bytes, FailureKind>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.done { return Poll::Ready(None); }
        let control = this.control.clone();
        control.body_waker.register(cx.waker());
        control.check_deadline();
        let (end, pull) = {
            let mut state = lock(&control.state);
            (state.end, state.pull.take())
        };
        if let Some(result) = end { return this.ended(result); }
        let Some(mut pull) = pull else {
            control.finish(Err(FailureKind::Internal));
            return this.ended(Err(FailureKind::Internal));
        };
        if !pull.drained {
            if pull.credit.is_none() {
                let acquiring = pull.acquiring.get_or_insert_with(|| {
                    control.budget.clone().acquire_many_owned(control.limits.chunk_bytes as u32).boxed()
                });
                match acquiring.as_mut().poll(cx) {
                    Poll::Pending => { control.restore(pull); return Poll::Pending; }
                    Poll::Ready(Err(_)) => return this.fail(pull, FailureKind::Internal),
                    Poll::Ready(Ok(credit)) => { pull.acquiring = None; pull.credit = Some(credit); }
                }
            }
            match pull.source.as_mut().poll_next(cx) {
                Poll::Pending => { control.restore(pull); return Poll::Pending; }
                Poll::Ready(Some(Err(_))) => return this.fail(pull, FailureKind::Body),
                Poll::Ready(Some(Ok(chunk))) => {
                    if chunk.len() > control.limits.chunk_bytes
                        || chunk.len() > control.limits.total_bytes.saturating_sub(lock(&control.state).emitted) {
                        return this.fail(pull, FailureKind::BodyTooLarge);
                    }
                    if chunk.is_empty() {
                        drop(pull.credit.take());
                        control.restore(pull);
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                    // Copy out of potentially larger native backing storage.
                    // Allocation is at most the already reserved chunk budget.
                    let bytes = Bytes::from_owner(ChargedBytes {
                        data: chunk.to_vec().into_boxed_slice(),
                        _credit: pull.credit.take().expect("reserved before upstream poll"),
                    });
                    drop(chunk);
                    control.check_deadline();
                    let stopped = {
                        let mut state = lock(&control.state);
                        if let Some(end) = state.end { Some(end) }
                        else { state.emitted += bytes.len(); None }
                    };
                    if let Some(end) = stopped {
                        drop((bytes, pull));
                        return this.ended(end);
                    }
                    control.restore(pull);
                    return Poll::Ready(Some(Ok(bytes)));
                }
                Poll::Ready(None) => {
                    pull.drained = true;
                    drop(pull.credit.take());
                }
            }
        }
        match Pin::new(&mut pull.cleanup).poll(cx) {
            Poll::Pending => { control.restore(pull); Poll::Pending }
            Poll::Ready(Err(_)) => this.fail(pull, FailureKind::Cleanup),
            Poll::Ready(Ok(native_owner::EndReason::Stopped(reason))) => this.fail(pull,
                if reason == native_owner::StopReason::Deadline { FailureKind::Deadline }
                else { FailureKind::Cancelled }),
            Poll::Ready(Ok(native_owner::EndReason::Response)) => {
                drop(pull);
                control.finish(Ok(()));
                let result = lock(&control.state).end.unwrap_or(Err(FailureKind::Internal));
                this.ended(result)
            }
        }
    }
}

impl Drop for Body {
    fn drop(&mut self) {
        if !self.done { self.control.cancel(); }
        drop(self.control.body_waker.take());
    }
}

struct Authorization {
    inner: Arc<dyn AttemptAuthorization>,
    control: Arc<Control>,
}

impl AttemptAuthorization for Authorization {
    fn authorize<'a>(&'a self, attempt: PreparedAttempt<'a>)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        async move {
            let observer = |cleanup: CloseObserver, cancellation: PreparationControl| {
                *lock(&self.control.preparation) = Some((cleanup, cancellation.clone()));
                if lock(&self.control.state).end.is_some() { cancellation.cancel(); }
            };
            self.inner.authorize_observed(attempt, &observer).await
        }.boxed()
    }

    fn observe_start(&self, cleanup: bridge::Cleanup, cancellation: native_owner::Cancellation) {
        *lock(&self.control.native) = Some((cleanup.clone(), cancellation.clone()));
        if lock(&self.control.state).end.is_some() { cancellation.cancel(); }
        self.inner.observe_start(cleanup, cancellation);
    }
}

async fn start(parts: &Parts, payload: Value, dialect: Dialect, step: u32, auth: &Authorization)
    -> Result<bridge::Pending, ApplicationError> {
    let invocation = parts.invocation(auth);
    invocation.start(&parts.request.identity.project, dialect, payload,
        &parts.request.identity.chat_session, step).await
}

fn account_cleanup_required(error: &ApplicationError) -> bool {
    matches!(error, ApplicationError::Start(ClientError::Draining
        | ClientError::Admission(native_owner::AdmissionError::SpawnFailed)
        | ClientError::Attachment))
}

async fn open(parts: Parts, payload: Value, dialect: Dialect,
    control: Arc<Control>, _hooks: Hooks) -> Result<RawStreamHead, DispatchError> {
    let auth = Authorization { inner: parts.authorization.clone(), control: control.clone() };
    #[cfg(test)]
    let started = match _hooks.start {
        Some(start) => {
            let started = start(&parts, payload);
            if let Ok(pending) = &started {
                auth.observe_start(pending.cleanup.clone(), pending.cancellation.clone());
            }
            started
        }
        None => start(&parts, payload, dialect, control.step_index, &auth).await,
    };
    #[cfg(not(test))]
    let started = start(&parts, payload, dialect, control.step_index, &auth).await;
    let pending = match started {
        Ok(pending) => pending,
        Err(error) => {
            if account_cleanup_required(&error) {
                lock(&control.state).require_scope_cleanup = true;
            }
            return Err(errors::error(error));
        }
    };
    let response = pending.response.await.map_err(|error| errors::error(ApplicationError::Bridge {
        error, cleanup: pending.cleanup.clone(),
    }))?;
    // Check the original String allocations before converting to shared exact
    // strings. The fallback shares the content-type allocation when present.
    let usage_bytes = response.usage_header.as_ref().map_or(0, String::capacity);
    let type_bytes = response.original_content_type.as_ref().map_or(
        parts.policy.missing_content_type.len(), String::capacity);
    if usage_bytes.checked_add(type_bytes).is_none_or(|n| n > control.limits.header_bytes) {
        return Err(DispatchError::new(FailureKind::BodyTooLarge));
    }
    let original_content_type = response.original_content_type.map(Arc::<str>::from);
    let headers = Arc::new(RawStreamHeaders {
        status: response.proxy.status,
        usage_header: response.usage_header.map(Arc::<str>::from),
        content_type: original_content_type.clone()
            .unwrap_or_else(|| Arc::from(parts.policy.missing_content_type.as_str())),
        original_content_type, elapsed: control.started.elapsed(),
    });
    let pull = Pull { source: response.proxy.stream, cleanup: pending.cleanup,
        credit: None, acquiring: None, drained: false };
    control.check_deadline();
    let failure = {
        let mut state = lock(&control.state);
        state.headers = Some(headers.clone());
        state.end.and_then(Result::err)
    };
    if let Some(kind) = failure { return Err(control.error(kind)); }
    control.restore(pull);
    Ok(RawStreamHead { headers, body: Box::pin(Body { control, done: false }) })
}

struct HeadReceiver {
    receiver: oneshot::Receiver<Result<RawStreamHead, DispatchError>>,
    control: Arc<Control>,
    done: bool,
}

impl Future for HeadReceiver {
    type Output = Result<RawStreamHead, DispatchError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.control.head_waker.register(cx.waker());
        self.control.check_deadline();
        let failure = lock(&self.control.state).end.and_then(Result::err);
        if let Some(kind) = failure {
            self.done = true;
            return Poll::Ready(Err(self.control.error(kind)));
        }
        match Pin::new(&mut self.receiver).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                self.control.check_deadline();
                self.done = true;
                let failure = lock(&self.control.state).end.and_then(Result::err);
                if let Some(kind) = failure {
                    drop(result);
                    Poll::Ready(Err(self.control.error(kind)))
                } else {
                    Poll::Ready(result.unwrap_or_else(|_| Err(self.control.error(FailureKind::Internal))))
                }
            }
        }
    }
}

impl Drop for HeadReceiver {
    fn drop(&mut self) {
        if !self.done { self.control.cancel(); }
        drop(self.control.head_waker.take());
    }
}

struct NoWork;
impl Cancellation for NoWork { fn cancel(&self) {} }

fn rejected(kind: FailureKind) -> RawStreamRun {
    RawStreamRun {
        response: InvocationRun { result: async move { Err(DispatchError::new(kind)) }.boxed(),
            cleanup: async { Ok(()) }.boxed().shared(), cancellation: Arc::new(NoWork),
            completed_attempts: None },
        finished: async move { RawStreamCompletion { result: Err(kind), headers: None,
            emitted_bytes: 0, elapsed: std::time::Duration::ZERO } }.boxed().shared(),
    }
}

pub(super) fn launch(call: Box<NativeCall>, payload: Value, limits: RawStreamLimits) -> RawStreamRun {
    launch_inner(call, payload, limits, Hooks::default(), None)
}

#[cfg(test)]
pub(super) fn launch_for_test(call: Box<NativeCall>, payload: Value,
    limits: RawStreamLimits, start: TestStart) -> RawStreamRun {
    launch_inner(call, payload, limits, Hooks { start: Some(start) }, None)
}

fn launch_inner(call: Box<NativeCall>, payload: Value, limits: RawStreamLimits,
    hooks: Hooks, prepared: Option<(Dialect, u32)>) -> RawStreamRun {
    if let Err(error) = limits.validate() { return rejected(error.kind); }
    if !payload.is_object() || payload.get("stream").is_some_and(|v| !v.is_boolean()) {
        return rejected(FailureKind::InvalidRequest);
    }
    let Route::Raw(dialect) = call.request.route else { return rejected(FailureKind::InvalidRequest); };
    let (dialect, step_index) = prepared.unwrap_or_else(|| (match dialect {
        ClientDialect::AnthropicMessages => Dialect::AnthropicMessages,
        ClientDialect::OpenAiChat => Dialect::OpenRouterChat,
        ClientDialect::OpenAiResponses => Dialect::OpenAiResponses,
    }, 0));
    let NativeCall { api, native, scope, authorization, request, policy, permit, account_lease } = *call;
    let account_lease = account_lease.into_running();
    let (stop, _) = watch::channel(None);
    let (ended, _) = watch::channel(false);
    let control = Arc::new(Control {
        state: Mutex::new(State { pull: None, headers: None, end: None,
            emitted: 0, elapsed: std::time::Duration::ZERO, require_scope_cleanup: false }),
        native: Mutex::new(None), preparation: Mutex::new(None), stop, ended,
        head_waker: AtomicWaker::new(), body_waker: AtomicWaker::new(),
        budget: Arc::new(Semaphore::new(limits.retained_bytes)), limits,
        started: Instant::now(), deadline: request.deadline, step_index,
    });
    let parts = Parts { api, native: native.clone(), scope: scope.clone(),
        authorization, request, policy };
    let (head_tx, head_rx) = oneshot::channel();
    let work_control = control.clone();
    let mut worker = tokio::spawn(async move {
        let result = tokio::select! {
            biased;
            _ = tokio::time::sleep_until(work_control.deadline) => {
                Err(work_control.error(FailureKind::Deadline))
            }
            kind = work_control.stopped() => Err(work_control.error(kind)),
            result = open(parts, payload, dialect, work_control.clone(), hooks) => result,
        };
        if let Err(error) = &result { work_control.finish(Err(error.kind)); }
        // Failure to hand off drops the body and cancels its native owner.
        let _ = head_tx.send(result);
    });
    let (finished_tx, finished_rx) = oneshot::channel();
    let (cleanup_tx, cleanup_rx) = oneshot::channel();
    let guarded = control.clone();
    let guardian = tokio::spawn(async move {
        // Both phases observe this handle. Shared cannot be polled after it
        // returns Ready; a finished body may also suppress the stop signal.
        let mut closed = scope.closed().fuse();
        let joined = tokio::select! {
            biased;
            _ = tokio::time::sleep_until(guarded.deadline) => {
                guarded.finish(Err(FailureKind::Deadline)); None
            }
            kind = guarded.stopped() => { guarded.finish(Err(kind)); None }
            result = &mut closed => {
                guarded.finish(Err(if result.is_err() { FailureKind::Cleanup }
                    else { FailureKind::ScopeClosed })); None
            }
            result = &mut worker => Some(result),
        };
        let worker_lost = match joined {
            Some(result) => result.is_err(),
            None => worker.await.is_err(),
        };
        if worker_lost { guarded.finish(Err(FailureKind::Internal)); }
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(guarded.deadline) => guarded.finish(Err(FailureKind::Deadline)),
            kind = guarded.stopped() => guarded.finish(Err(kind)),
            result = &mut closed => guarded.finish(Err(if result.is_err() { FailureKind::Cleanup }
                else { FailureKind::ScopeClosed })),
            _ = guarded.body_finished() => {},
        }
        // Report body failure before any potentially delayed native cleanup.
        let _ = finished_tx.send(guarded.summary(false));
        let native_cleanup = lock(&guarded.native).as_ref().map(|(c, _)| c.clone());
        let native_failed = match native_cleanup {
            Some(cleanup) => cleanup.await.is_err(), None => false,
        };
        let require_scope_cleanup = lock(&guarded.state).require_scope_cleanup;
        // Err can be published while the account still retains live requests.
        // Only Ok proves the entire generation was released by its supervisor.
        let scope_failed = (require_scope_cleanup && scope.closed().await.is_err())
            || account_lease.scope_failed(&scope);
        let preparation = lock(&guarded.preparation).as_ref().map(|(c, _)| c.clone());
        let preparation_failed = match preparation {
            Some(cleanup) => cleanup.await.is_err(), None => false,
        };
        let failed = worker_lost || native_failed || scope_failed || preparation_failed;
        if failed {
            // The parked guardian must not hide an unconfirmed account owner.
            drop(account_lease);
            let _ = cleanup_tx.send(Err(FailureKind::Cleanup));
            // A failed original completion or lost worker cannot free capacity.
            futures::future::pending::<()>().await;
            drop((permit, scope, native));
            return;
        }
        // Returned Bytes may outlive EOF or body abandonment. Their original
        // credits must return before this invocation can admit another one.
        let all_credits = match guarded.budget.clone()
            .acquire_many_owned(limits.retained_bytes as u32).await {
            Ok(credit) => credit,
            Err(_) => {
                drop(account_lease);
                let _ = cleanup_tx.send(Err(FailureKind::Cleanup));
                futures::future::pending::<()>().await;
                drop((permit, scope, native));
                return;
            }
        };
        if let Err(kind) = account_lease.complete(&scope) {
            let _ = cleanup_tx.send(Err(kind));
            futures::future::pending::<()>().await;
            drop((all_credits, permit, scope, native));
            return;
        }
        drop((all_credits, permit, scope, native));
        let _ = cleanup_tx.send(Ok(()));
    });
    let cleanup = async move {
        match cleanup_rx.await {
            Ok(Ok(())) => guardian.await.map_err(|_| FailureKind::Cleanup),
            Ok(Err(kind)) => { drop(guardian); Err(kind) }
            Err(_) => { let _ = guardian.await; Err(FailureKind::Cleanup) }
        }
    }.boxed().shared();
    let terminal = control.clone();
    let finished = async move {
        finished_rx.await.unwrap_or_else(|_| terminal.summary(true))
    }.boxed().shared();
    let progress = control.clone();
    let completed_attempts: framer_core::native_dispatch::AttemptProgress = Arc::new(move || {
        let ended = lock(&progress.state).end.is_some();
        if ended { progress.error(FailureKind::Internal).attempts } else { Vec::new() }
    });
    RawStreamRun { response: InvocationRun {
        result: Box::pin(HeadReceiver { receiver: head_rx, control: control.clone(), done: false }),
        cleanup, cancellation: control, completed_attempts: Some(completed_attempts),
    }, finished }
}

// Internal prepared JSON attempt. The public raw route remains unchanged.
pub(super) fn launch_prepared(call: Box<NativeCall>, payload: Value,
    limits: RawStreamLimits, dialect: Dialect, step: u32) -> RawStreamRun {
    launch_inner(call, payload, limits, Hooks::default(), Some((dialect, step)))
}

#[cfg(test)]
pub(super) fn launch_prepared_for_test(call: Box<NativeCall>, payload: Value,
    limits: RawStreamLimits, dialect: Dialect, step: u32, start: TestStart) -> RawStreamRun {
    launch_inner(call, payload, limits, Hooks { start: Some(start) }, Some((dialect, step)))
}
