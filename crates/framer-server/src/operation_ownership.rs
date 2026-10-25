//! Opt-in ownership wiring. Existing unmanaged host work is a permanent barrier
//! until a separate observed close integration exists. Never claim Held from
//! HTTP completion or local counters alone.
use axum::{body::Body, extract::{Request, State}, middleware::Next,
    response::{IntoResponse, Response}, http::StatusCode};
use framer_core::{operation_gate::{OperationGate, OperationOwner, OperationKind,
    GateError, DrainTicket, ControlGuard, LegacyHostOwner, ApplicationDrained}, wave::WaveGuard};
use std::{future::Future, pin::Pin, sync::{Arc, Mutex}, task::{Context, Poll}};
use http_body::{Body as HttpBody, Frame, SizeHint};
use futures::{future::{BoxFuture, Shared}, FutureExt};

tokio::task_local! { static CURRENT: Arc<OwnerContext>; }
tokio::task_local! { static CLIENT: tokio::sync::watch::Receiver<()>; }

/// Cancels future attempts when the HTTP waiter disappears, not current work.
pub(crate) fn request_cancelled() -> Option<Arc<dyn Fn() -> bool + Send + Sync>> {
    CLIENT.try_with(|client| {
        let client = client.clone();
        Arc::new(move || client.has_changed().is_err()) as Arc<dyn Fn() -> bool + Send + Sync>
    }).ok()
}

/// The HTTP waiter owns the only sender. Its drop wakes local admission waits.
/// No value is sent, so closure is the only notification; polling cannot miss it.
pub(crate) async fn request_disconnected() {
    match CLIENT.try_with(Clone::clone) {
        Ok(mut client) => { let _ = client.changed().await; }
        Err(_) => std::future::pending::<()>().await,
    }
}

/// Correlate only opted-in work. Client headers never supply this identity.
pub(crate) fn correlate_chain(event: framer_core::observability::Event)
    -> framer_core::observability::Event {
    match CURRENT.try_with(|owner| owner.request_id.clone()) {
        Ok(id) => event.request_id(&id),
        Err(_) => event,
    }
}

pub struct ServingOwnership {
    gate: OperationGate,
    _guard: ControlGuard,
    // No release API: existing raw BrowserBridge cannot supply closure proof.
    _unmanaged_host: LegacyHostOwner,
    #[cfg(test)]
    observation_sink: Mutex<Option<Arc<dyn Fn(serde_json::Value) + Send + Sync>>>,
}
impl ServingOwnership {
    /// Construct host ownership with this exact gate. The retained host barrier
    /// is unchanged; this does not attest that the host can be safely retired.
    /// The factory should construct an inert host before any serving work starts.
    pub fn new_with_host_factory<T>(
        wave: Arc<WaveGuard>, limit: usize,
        make_host: impl FnOnce(OperationGate) -> T,
    ) -> Result<(Arc<Self>, T), GateError> {
        let ownership = Self::new_unmanaged(wave, limit)?;
        let host = make_host(ownership.gate.clone());
        Ok((ownership, host))
    }

    pub fn new_unmanaged(wave: Arc<WaveGuard>, limit: usize) -> Result<Arc<Self>, GateError> {
        let (gate, host) = OperationGate::new_with_legacy_host(limit)?;
        Ok(Arc::new(Self { _guard: ControlGuard::bind(gate.clone(), wave),
            gate, _unmanaged_host: host,
            #[cfg(test)]
            observation_sink: Mutex::new(None),
        }))
    }
    /// Seal rejects new work, but wait cannot reach Held while unmanaged host
    /// ownership persists. External close remains an explicit unsatisfied fact.
    pub fn seal(&self) -> Result<DrainTicket, GateError> { self.gate.seal() }
    pub fn unmanaged_host_close_required(&self) -> bool { true }
    /// Registered application completion is distinct from host retirement.
    pub fn check_application_drained(&self, proof: &ApplicationDrained<'_>)
        -> Result<(), GateError> {
        self.gate.check_application_drained(proof)
    }
    fn owner(self: &Arc<Self>, kind: OperationKind) -> Result<Arc<OwnerContext>, GateError> {
        let owner = self.registration(kind)?;
        let operation_id = uuid::Uuid::new_v4().to_string();
        let parent = CURRENT.try_with(|parent|
            (parent.request_id.clone(), parent.operation_id.clone())).ok();
        let context = Arc::new(OwnerContext {
            kind: owner.kind(), request_id: parent.as_ref().map(|p| p.0.clone())
                .unwrap_or_else(|| operation_id.clone()),
            parent_operation_id: parent.map(|p| p.1), operation_id,
            owner: Mutex::new(Some(owner)), scope: self.clone(),
            preflight_completed: std::sync::atomic::AtomicBool::new(false),
        });
        context.observe("operation.registered", "active", None);
        Ok(context)
    }
    fn registration(self: &Arc<Self>, kind: OperationKind) -> Result<OperationOwner, GateError> {
        match CURRENT.try_with(|parent| {
            if !Arc::ptr_eq(&parent.scope, self) { return Err(GateError::WrongGate); }
            parent.child()
        }) {
            Ok(value) => value,
            Err(_) => self.gate.admit(kind),
        }
    }
    pub(crate) fn external_cleanup(self: &Arc<Self>) -> Result<OperationOwner, GateError> {
        self.registration(OperationKind::Inference)
    }
}

struct OwnerContext {
    owner: Mutex<Option<OperationOwner>>, scope: Arc<ServingOwnership>,
    request_id: String, operation_id: String, parent_operation_id: Option<String>,
    kind: OperationKind,
    preflight_completed: std::sync::atomic::AtomicBool,
}

/// Preserve typed errors while attesting a terminal refusal before any
/// residency acquisition or external work. Never use for retry admission.
pub(crate) fn completed_preflight<E>(error: E) -> E {
    let _ = CURRENT.try_with(|owner| owner.preflight_completed.store(
        true, std::sync::atomic::Ordering::Relaxed));
    error
}
impl OwnerContext {
    fn observe(&self, event: &'static str, outcome: &'static str, confirmed: Option<bool>) {
        // Diagnostics must never turn a completed operation into a panic/drop.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let fields = serde_json::json!({
                "event": event, "request_id": self.request_id,
                "operation_id": self.operation_id,
                "parent_operation_id": self.parent_operation_id,
                "kind": format!("{:?}", self.kind), "outcome": outcome,
                "cleanup_confirmed": confirmed,
            });
            #[cfg(test)]
            {
                let sink = self.scope.observation_sink.lock().ok()
                    .and_then(|slot| slot.clone());
                if let Some(sink) = sink { sink(fields.clone()); }
            }
            let mut record = framer_core::observability::Event::new(event);
            if let Some(fields) = fields.as_object() {
                for (key, value) in fields { record = record.field(key, value.clone()); }
            }
            record.emit();
        }));
    }
    fn child(&self) -> Result<OperationOwner, GateError> {
        self.owner.lock().map_err(|_| GateError::Poisoned)?.as_ref()
            .ok_or(GateError::Unconfirmed)?.register_cleanup()
    }
    fn finish(&self) -> bool {
        let owner = match self.owner.lock() {
            Ok(mut slot) => slot.take(),
            Err(_) => return false,
        };
        let Some(owner) = owner else { return false; };
        let completed = owner.complete().is_ok();
        if completed {
            self.observe("operation.released", "confirmed", Some(true));
        } else {
            self.observe("operation.unresolved", "completion_error", Some(false));
        }
        completed
    }
}
impl Drop for OwnerContext {
    fn drop(&mut self) {
        let owner = self.owner.get_mut().unwrap_or_else(|e| e.into_inner()).take();
        if owner.is_some() {
            // Set the gate fence before reporting unresolved registration.
            drop(owner);
            self.observe("operation.unresolved", "dropped", Some(false));
        }
    }
}

/// All four serving detached jobs and startup prewarm use this boundary.
/// Registration happens BEFORE spawn. Caller cancellation cannot discard an
/// admitted detached job; explicit existing producer abort remains unconfirmed.
pub fn spawn<T, F>(scope: Option<&Arc<ServingOwnership>>, kind: OperationKind, future: F)
    -> Result<tokio::task::JoinHandle<T>, GateError>
where T: Send + 'static, F: Future<Output = T> + Send + 'static {
    let Some(scope) = scope else { return Ok(tokio::spawn(future)); };
    let owner = scope.owner(kind)?;
    Ok(tokio::spawn(CURRENT.scope(owner.clone(), async move {
        let result = future.await;
        owner.finish();
        result
    })))
}

#[derive(Clone)]
pub(crate) struct ResponseCompletion(Shared<BoxFuture<'static, bool>>);

type JoinedResult<T> = Result<Result<T, String>, tokio::task::JoinError>;

/// The body alone takes the original result. Its cleanup observer shares only
/// a verdict, so neither T nor JoinError needs to be cloned or substituted.
pub(crate) struct ObservedJoin<T> {
    result: BoxFuture<'static, JoinedResult<T>>,
    #[cfg(test)]
    abort: tokio::task::AbortHandle,
}
impl<T> Future for ObservedJoin<T> {
    type Output = JoinedResult<T>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().result.as_mut().poll(cx)
    }
}
#[cfg(test)]
impl<T> ObservedJoin<T> {
    fn abort_handle(&self) -> tokio::task::AbortHandle { self.abort.clone() }
}

/// A detached operation reports its own cleanup even after its parent's HTTP
/// body has gone. Aborting or losing the verdict leaves this child unconfirmed.
pub(crate) fn spawn_with_cleanup<T, F>(scope: Option<&Arc<ServingOwnership>>,
    kind: OperationKind, future: F) -> Result<tokio::task::JoinHandle<T>, GateError>
where T: Send + 'static, F: Future<Output = (T, bool)> + Send + 'static {
    let Some(scope) = scope else {
        return Ok(tokio::spawn(async move { future.await.0 }));
    };
    let owner = scope.owner(kind)?;
    Ok(tokio::spawn(CURRENT.scope(owner.clone(), async move {
        let (result, confirmed) = future.await;
        if confirmed { owner.finish(); }
        result
    })))
}

impl ResponseCompletion {
    pub(crate) fn from_receiver(receiver: tokio::sync::oneshot::Receiver<bool>) -> Self {
        Self(async move { receiver.await.unwrap_or(false) }.boxed().shared())
    }
}

/// Only for an already assembled response with an observed cleanup outcome.
pub(crate) fn completed_response(mut response: Response, confirmed: bool) -> Response {
    response.extensions_mut().insert(ResponseCompletion(
        futures::future::ready(confirmed).boxed().shared()));
    response
}

/// Observe the existing producer task. Cooperative cancellation must stop its
/// next attempt; this signal only resolves after its in-flight work returns.
/// The producer retains cleanup evidence from every attempt separately from
/// its final client result.
pub(crate) fn spawn_observed<T, F>(scope: Option<&Arc<ServingOwnership>>,
    kind: OperationKind, future: F)
    -> Result<(ObservedJoin<T>, ResponseCompletion), GateError>
where T: Send + 'static, F: Future<Output = (Result<T, String>, bool)> + Send + 'static {
    let owner = scope.map(|scope| scope.owner(kind)).transpose()?;
    let task = match &owner {
        Some(owner) => tokio::spawn(CURRENT.scope(owner.clone(), future)),
        None => tokio::spawn(future),
    };
    #[cfg(test)]
    let abort = task.abort_handle();
    let (send_result, receive_result) = tokio::sync::oneshot::channel();
    let completion = observe_join(task, owner, send_result).boxed().shared();
    let joined = completion.clone();
    let result = async move {
        joined.await;
        receive_result.await.expect("original join result stored before completion")
    }.boxed();
    Ok((ObservedJoin { result, #[cfg(test)] abort }, ResponseCompletion(completion)))
}

// Polled by the body or its retained disposal observer. There is one original
// producer handle, and its child registration outlives that handle's join.
async fn observe_join<T>(task: tokio::task::JoinHandle<(Result<T, String>, bool)>,
    owner: Option<Arc<OwnerContext>>, send_result: tokio::sync::oneshot::Sender<JoinedResult<T>>)
    -> bool {
    let (result, mut confirmed, outcome) = match task.await {
        Ok((result, cleanup_confirmed)) => {
            let confirmed = cleanup_confirmed && result.as_ref().err()
                .map_or(true, |error| !crate::pool_control::cleanup_unconfirmed(error));
            let outcome = if result.is_ok() { "ok" } else { "error" };
            (Ok(result), confirmed, outcome)
        }
        Err(error) => {
            let outcome = if error.is_cancelled() { "cancelled" } else { "panicked" };
            (Err(error), false, outcome)
        }
    };
    if let Some(owner) = &owner {
        owner.observe("operation.joined", outcome, Some(confirmed));
        if confirmed { confirmed = owner.finish(); }
    }
    drop(owner); // An uncertain child fences admission before completion resolves.
    let _ = send_result.send(result);
    confirmed
}

pub async fn run<T, F>(scope: Option<&Arc<ServingOwnership>>, kind: OperationKind, future: F)
    -> Result<T, GateError>
where T: Send + 'static, F: Future<Output = T> + Send + 'static {
    if scope.is_none() { return Ok(future.await); }
    spawn(scope, kind, future)?.await.map_err(|_| GateError::Unconfirmed)
}

pub async fn middleware(State(scope): State<Option<Arc<ServingOwnership>>>,
    request: Request, next: Next) -> Response {
    let Some(scope) = scope else { return next.run(request).await; };
    if request.uri().path() == "/api/health" { return next.run(request).await; }
    let path = request.uri().path();
    let kind = if path.contains("/accounts/") || path.starts_with("/api/pool/") {
        OperationKind::Admin
    } else { OperationKind::Inference };
    let owner = match scope.owner(kind) {
        Ok(owner) => owner,
        Err(error) => {
            let mut event = framer_core::observability::Event::new("operation.admission.refused")
                .field("reason", format!("{error:?}"))
                .field("kind", format!("{kind:?}"));
            if let Ok(stats) = scope.gate.stats() {
                event = event.field("active", stats.active).field("sealed", stats.sealed)
                    .field("unconfirmed", stats.unconfirmed);
            }
            event.emit();
            return (StatusCode::SERVICE_UNAVAILABLE, "operation admission closed").into_response();
        },
    };
    let request_id = owner.request_id.clone();
    let (present, client) = tokio::sync::watch::channel(());
    let task = tokio::spawn(CLIENT.scope(client, CURRENT.scope(owner.clone(), async move {
        // Handler remains owned even when the HTTP result waiter disappears.
        // Auth/session/state mutation futures are not cancelled by that drop.
        let response = next.run(request).await;
        let (parts, body) = response.into_parts();
        let completion = parts.extensions.get::<ResponseCompletion>().cloned().or_else(|| {
            owner.preflight_completed.load(std::sync::atomic::Ordering::Relaxed)
                .then(|| ResponseCompletion(futures::future::ready(true).boxed().shared()))
        });
        Response::from_parts(parts, Body::new(OwnedBody { body, owner: Some(owner),
            completion,
        }))
    })));
    let result = task.await;
    drop(present);
    let mut response = match result {
        Ok(response) => response,
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "operation completion unconfirmed").into_response(),
    };
    if let Ok(value) = request_id.parse() {
        response.headers_mut().insert("x-framerize-operation-id", value);
    }
    response
}

struct OwnedBody {
    body: Body, owner: Option<Arc<OwnerContext>>,
    completion: Option<ResponseCompletion>,
}
impl OwnedBody {
    fn finish(&mut self) {
        if self.observe_completion() { return; }
        if let Some(owner) = self.owner.take() { owner.finish(); }
    }
    fn observe_completion(&mut self) -> bool {
        if let Some(completion) = self.completion.take() {
            // Body drop cancels future attempts. Retain this owner until the
            // admitted producer (and native cleanup, when present) joins.
            drop(std::mem::replace(&mut self.body, Body::empty()));
            if let Some(owner) = self.owner.take() {
                if let Some(confirmed) = completion.0.clone().now_or_never() {
                    if confirmed { owner.finish(); }
                } else if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    runtime.spawn(async move {
                        if completion.0.await { owner.finish(); }
                    });
                }
            }
            return true;
        }
        false
    }
}
impl HttpBody for OwnedBody {
    type Data = axum::body::Bytes;
    type Error = axum::Error;
    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
        -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let value = Pin::new(&mut self.body).poll_frame(cx);
        match &value {
            Poll::Ready(None) => self.finish(),
            Poll::Ready(Some(Err(_))) => {
                if !self.observe_completion() { self.owner.take(); }
            },
            _ => {},
        }
        value
    }
    fn is_end_stream(&self) -> bool { self.body.is_end_stream() }
    fn size_hint(&self) -> SizeHint { self.body.size_hint() }
}
impl Drop for OwnedBody {
    fn drop(&mut self) {
        if self.observe_completion() { return; }
        // Without a cleanup observer, early drop remains unconfirmed. In
        // particular, do not drain abandoned browser chains: drop must abort
        // them before they can issue another mint or continuation.
        if self.body.is_end_stream() { self.finish(); }
    }
}

#[cfg(test)]
#[path = "operation_ownership_tests.rs"]
mod tests;

/// Explicit construction seam for a future reviewed startup controller. No
/// CLI or default calls it. Bind before proxy loading, prewarm or serving.
pub fn with_owned_operations(mut state: crate::AppState, limit: usize)
    -> Result<crate::AppState, GateError> {
    if state.operation_ownership.is_some() { return Err(GateError::Unconfirmed); }
    state.operation_ownership = Some(ServingOwnership::new_unmanaged(state.wave.clone(), limit)?);
    Ok(state)
}

// Test-only observation at the real access_log emission site. No global logger
// initialization or private/log-filesystem access is needed by these fixtures.
#[cfg(test)]
#[derive(Clone)]
pub(crate) struct AccessLogCapture(pub Arc<Mutex<Vec<u16>>>);
