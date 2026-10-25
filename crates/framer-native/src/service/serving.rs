//! One complete-JSON request using the live pool and its existing owners.
use crate::authorization::CredentialTarget;
use crate::authorization::control::OfflineMintTarget;
use crate::account::client::AccountScope;
use crate::account::client::NativeAccountClient;
use crate::transport::http::NativeEgress;
use crate::transport::http::NativeHttpLimits;
use crate::account::sessions::OpenAccountError;
use crate::request::complete_json::CompletedJson;
use crate::request::complete_json::Evidence;
use crate::request::complete_json::FreeRoute;
use crate::request::complete_json::Plan;
use crate::request::complete_json::ProbeRun;
use crate::request::complete_json::Refusal;
use crate::request::complete_json::Specification;
use framer_core::convert;
use crate::request::types::self as port;
use crate::request::types::DispatchError;
use crate::request::types::FailureKind;
use framer_server::AppState;
use framer_server::admission as native_control;
use framer_server::admission::AdmittedControl;
use framer_server::admission::HealthOutcome;
use futures::future::BoxFuture;
use futures::FutureExt;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::time::Instant;
use crate::service::source::OfflineSource;
use crate::worker::backend::WorkerBackend;
use crate::authorization::source::SourceBuilder;
use crate::service::sessions::Sessions;
use crate::service::device::DeviceLease;
use crate::account::device_owner::DeviceOwner;

#[cfg(test)]
#[path = "offline_serving_tests.rs"]
mod tests;

pub(crate) struct Request { spec: Specification, target: OfflineMintTarget }
impl Request {
    pub(crate) fn new(spec: Specification, page_url: String) -> Result<Self, DispatchError> {
        Plan::validate(&spec)?;
        let path = match spec.route { FreeRoute::Responses => "/ai/v3/responses/",
            FreeRoute::Messages => "/ai/v3/messages/" };
        let target = OfflineMintTarget::new(CredentialTarget {
            project: spec.identity.project.clone(), path,
        }, page_url).map_err(|_| DispatchError::new(FailureKind::InvalidRequest))?;
        Ok(Self { spec, target })
    }
}

pub(crate) struct Outcome {
    pub response: Option<serde_json::Value>,
    pub evidence: Option<Evidence>,
    pub failure: Option<FailureKind>,
    pub cleanup_confirmed: bool,
}

impl Outcome {
    fn failed(kind: FailureKind) -> Self {
        Self { response: None, evidence: None, failure: Some(kind), cleanup_confirmed: true }
    }
    fn unconfirmed() -> Self {
        Self { cleanup_confirmed: false, ..Self::failed(FailureKind::Cleanup) }
    }
    fn reject(&mut self, kind: FailureKind) {
        self.response = None;
        self.failure = Some(if self.cleanup_confirmed { kind } else { FailureKind::Cleanup });
    }
}

struct Stop(watch::Sender<bool>);
impl Stop {
    fn cancel(&self) { self.0.send_replace(true); }
    fn is_cancelled(&self) -> bool { *self.0.borrow() }
    async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        while !*receiver.borrow_and_update() {
            if receiver.changed().await.is_err() { break; }
        }
    }
}
struct CancelOnDrop(Arc<Stop>);
impl Drop for CancelOnDrop { fn drop(&mut self) { self.0.cancel(); } }

#[must_use = "retain cleanup after cancellation or dropping the result"]
pub(crate) struct Run {
    pub result: BoxFuture<'static, Outcome>,
    pub cleanup: port::Cleanup,
    stop: Arc<Stop>,
}
impl Run { pub(crate) fn cancel(&self) { self.stop.cancel(); } }

pub(crate) fn launch(state: AppState, request: Request, worker: impl Into<WorkerBackend>,
    sessions: Sessions, device_owner: Arc<dyn DeviceOwner>)
    -> Result<Run, DispatchError> {
    let source = Arc::new(OfflineSource::from_worker(worker.into(), request.target, device_owner));
    launch_with_source(state, request.spec, source, sessions)
}

fn launch_with_source(state: AppState, spec: Specification, source: Arc<dyn SourceBuilder>,
    sessions: Sessions)
    -> Result<Run, DispatchError> {
    Plan::validate(&spec)?;
    if !sessions.matches(&state.sessions, state.operation_ownership.as_ref()) {
        return Err(DispatchError::new(FailureKind::InvalidScope));
    }
    let ownership = state.operation_ownership.clone()
        .ok_or_else(|| DispatchError::new(FailureKind::Ineligible))?;
    let stop = Arc::new(Stop(watch::channel(false).0));
    let cancel = stop.clone();
    let (sender, receiver) = oneshot::channel();
    let task = framer_server::operation_ownership::spawn(Some(&ownership),
        framer_core::operation_gate::OperationKind::Inference, async move {
            let outcome = owned(state, spec, source, cancel, sessions).await;
            let cleanup = if outcome.cleanup_confirmed { Ok(()) } else { Err(FailureKind::Cleanup) };
            let _ = sender.send(outcome);
            cleanup
        }).map_err(|_| DispatchError::new(FailureKind::Ineligible))?;
    let cleanup = async move { task.await.unwrap_or(Err(FailureKind::Cleanup)) }.boxed().shared();
    let completed = cleanup.clone();
    let guard = CancelOnDrop(stop.clone());
    let result = async move {
        let _guard = guard;
        let mut outcome = receiver.await.unwrap_or_else(|_| Outcome::unconfirmed());
        if completed.await.is_err() {
            outcome.cleanup_confirmed = false;
            outcome.reject(FailureKind::Cleanup);
        }
        outcome
    }.boxed();
    Ok(Run { result, cleanup, stop })
}

fn stopped(stop: &Stop, deadline: Instant) -> Option<FailureKind> {
    if Instant::now() >= deadline { Some(FailureKind::Deadline) }
    else if stop.is_cancelled() { Some(FailureKind::Cancelled) } else { None }
}

async fn owned(state: AppState, spec: Specification, source: Arc<dyn SourceBuilder>,
    stop: Arc<Stop>, sessions: Sessions) -> Outcome {
    let deadline = spec.deadline;
    if let Some(kind) = stopped(&stop, spec.deadline) { return Outcome::failed(kind); }
    let model = spec.payload["model"].as_str().expect("validated model");
    let admitted = match native_control::admit(&state, spec.identity.clone(), model, spec.deadline).await {
        Ok(value) => value, Err(error) => return Outcome::failed(error.into()),
    };
    if let Some(kind) = stopped(&stop, spec.deadline) { return Outcome::failed(kind); }
    let AdmittedControl { identity, proxy, capacity, pool, mut completion } = admitted;
    completion.start();
    let mut outcome = session(&state, spec, source, (capacity, pool), &completion,
        &proxy, stop.clone(), &sessions).await;
    let health = match &outcome.evidence {
        Some(e) if e.status == Some(403) && e.attempts == 1
            && e.refusal.is_some_and(|r| r.policy == Refusal::Castle) => HealthOutcome::RiskRefused,
        Some(e) if e.accepted => HealthOutcome::Complete,
        _ => HealthOutcome::Unclassified,
    };
    if let Err(error) = completion.publish(&state, &identity, &proxy, health).await {
        outcome.reject(error.into());
    }
    if let Some(kind) = stopped(&stop, deadline) { outcome.reject(kind); }
    if outcome.cleanup_confirmed && completion.complete().is_err() {
        outcome.cleanup_confirmed = false;
        outcome.reject(FailureKind::Cleanup);
    }
    outcome
}

type Admission = (tokio::sync::OwnedSemaphorePermit, port::PoolAdmission);

async fn execute(state: &AppState, spec: Specification, source: Arc<dyn SourceBuilder>,
    admission: Admission, owner: &native_control::ControlOwner, proxy: &Option<String>,
    stop: Arc<Stop>, native: &NativeAccountClient, scope: &AccountScope,
    device: Option<Arc<crate::account::device::DeviceGrant>>) -> Outcome {
    if let Some(kind) = stopped(&stop, spec.deadline) { return Outcome::failed(kind); }
    if let Err(error) = owner.revalidate(state, &spec.identity, proxy).await {
        return Outcome::failed(error.into());
    }
    let deadline = spec.deadline;
    let model = spec.payload["model"].as_str().expect("validated model").to_owned();
    let plan = match Plan::new(scope.clone(), spec) {
        Ok(value) => value, Err(error) => return Outcome::failed(error.kind),
    };
    let source = match source.build_with_device(scope.clone(), proxy.clone(), device) {
        Ok(value) => value, Err(_) => return Outcome::failed(FailureKind::Authorization),
    };
    if let Some(kind) = stopped(&stop, deadline) { return Outcome::failed(kind); }
    let probe = match plan.launch_with_source_in_pool(
        native.clone(), source, admission.0, admission.1).await {
        Ok(value) => value, Err(error) => return Outcome::failed(error.kind),
    };
    let cleanup = probe.cleanup.clone();
    let mut outcome = AssertUnwindSafe(observe(probe, stop, deadline, &model))
        .catch_unwind().await.unwrap_or_else(|_| Outcome::unconfirmed());
    if cleanup.await.is_err() {
        outcome.cleanup_confirmed = false;
        outcome.reject(FailureKind::Cleanup);
    }
    outcome
}

async fn observe(mut probe: ProbeRun, stop: Arc<Stop>, deadline: Instant,
    model: &str) -> Outcome {
    let mut cancelled = None;
    let evidence = tokio::select! {
        biased;
        _ = stop.cancelled() => {
            cancelled = Some(FailureKind::Cancelled);
            probe.cancellation.cancel();
            probe.result.await
        }
        _ = tokio::time::sleep_until(deadline) => {
            cancelled = Some(FailureKind::Deadline);
            probe.cancellation.cancel();
            probe.result.await
        }
        evidence = &mut probe.result => evidence,
    };
    let completed = probe.completed.await;
    let response = if evidence.accepted && cancelled.is_none() {
        completed.map(|value| match value {
            CompletedJson::Responses(value) => convert::responses_json(&value, model, &value.id),
            CompletedJson::Messages(value) =>
                convert::render_json(convert::ClientDialect::AnthropicMessages, &value, model),
        })
    } else { None };
    let failure = cancelled.or(evidence.failure).or_else(||
        response.is_none().then_some(FailureKind::UnsupportedResponse));
    Outcome { response, cleanup_confirmed: evidence.cleanup_confirmed,
        evidence: Some(evidence), failure }
}

async fn session(state: &AppState, spec: Specification, source: Arc<dyn SourceBuilder>,
    admission: Admission, owner: &native_control::ControlOwner, proxy: &Option<String>,
    stop: Arc<Stop>, sessions: &Sessions) -> Outcome {
    use crate::account::device::DeviceFailure;
    let device = match DeviceLease::open(source.as_ref(), spec.identity.account).await {
        Ok(device) => device,
        Err(DeviceFailure::Rejected) => return Outcome::failed(FailureKind::Ineligible),
        Err(DeviceFailure::Unconfirmed) => {
            admission.0.forget();
            return Outcome::unconfirmed();
        }
    };
    let mut outcome = if let Some(kind) = stopped(&stop, spec.deadline) {
        Outcome::failed(kind)
    } else {
        AssertUnwindSafe(session_inner(state, spec, source, admission, owner, proxy,
            stop, sessions, device.grant())).catch_unwind().await
            .unwrap_or_else(|_| Outcome::unconfirmed())
    };
    let risk = outcome.evidence.as_ref().is_some_and(|e| e.status == Some(403)
        && e.attempts == 1 && e.refusal.is_some_and(|r| r.policy == Refusal::Castle));
    if !device.finish(outcome.cleanup_confirmed, risk).await {
        outcome.cleanup_confirmed = false;
        outcome.reject(FailureKind::Cleanup);
    }
    outcome
}

async fn session_inner(state: &AppState, spec: Specification, source: Arc<dyn SourceBuilder>,
    admission: Admission, owner: &native_control::ControlOwner, proxy: &Option<String>,
    stop: Arc<Stop>, sessions: &Sessions,
    device: Option<Arc<crate::account::device::DeviceGrant>>) -> Outcome {
    let native = sessions.native();
    let bridge = sessions.bridge();
    let route = proxy.as_deref().map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct);
    // Do not drop a private-store operation on cancellation. Its owner waits
    // for the actual read, then closes the resulting scope without issuing.
    let opened = AssertUnwindSafe(bridge.open_account_observed(spec.identity.account,
        spec.identity.user.clone(), NativeHttpLimits::default(), route)).catch_unwind().await;
    match opened {
        Ok(Ok(scope)) => {
            let mut outcome = AssertUnwindSafe(execute(state, spec, source, admission,
                owner, proxy, stop, native, &scope, device)).catch_unwind().await
                .unwrap_or_else(|_| Outcome::unconfirmed());
            let closed = match native.close_account(&scope) {
                Ok(observer) => observer.await.is_ok(), Err(_) => false,
            };
            if !closed { outcome.cleanup_confirmed = false; outcome.reject(FailureKind::Cleanup); }
            outcome
        }
        Ok(Err(OpenAccountError::Rejected(_))) => Outcome::failed(FailureKind::Authorization),
        Ok(Err(OpenAccountError::CleanupUnconfirmed)) | Err(_) => Outcome::unconfirmed(),
    }
}
