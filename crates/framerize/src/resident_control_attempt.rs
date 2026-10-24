use super::*;
use framer_api::experimental_native::{
    authorization::PreparedCredentials,
    control_authorization::{BoundControlClient, resident_control::{
        ResidentGrant, ResidentObservation, TrustedResidentContext}},
    native_http::{AccountRequestAuth, NativeCancellation, NativeHttp, NativeTrackedRequest},
};
use framer_core::port::Session;
use framer_host::sidecar::{observer::ApplicationGeneration, resident::{
    ResidentAcquire, ResidentAdmission, ResidentDiscard, ResidentIdentity, ResidentRpc,
    MAX_BUDGET_MS}};
use futures::StreamExt;
use serde_json::Value;

pub(super) struct Operation {
    capture: Arc<Capture>,
    binding: wire::Binding,
    rpc: ResidentRpc,
    pub acquire_started: AtomicBool,
}
impl Operation {
    pub async fn bind(capture: Arc<Capture>) -> Result<Self, Failure> {
        let page = &capture.page;
        let binding = wire::Binding { account: capture.identity.account,
            application: page.application_generation, host: page.host_generation.clone(),
            incarnation: page.account_incarnation, lease: uuid::Uuid::new_v4() };
        binding.api().map_err(|_| Failure::Context)?;
        let identity = ResidentIdentity::new(binding.account,
            ApplicationGeneration(binding.application), binding.host.clone(),
            binding.incarnation, binding.lease).map_err(|_| Failure::Context)?;
        let rpc = ResidentRpc::bind(&capture.sidecar, identity).await
            .map_err(|_| Failure::MissingOwner)?;
        Ok(Self { capture, binding, rpc, acquire_started: AtomicBool::new(false) })
    }
    pub async fn settle(&self, native_settled: bool) -> bool {
        // Automatic resumption needs a reviewed policy. Keep host admission
        // fenced on both outcomes; this does not certify inference success.
        let reply = if !native_settled {
            self.rpc.discard(ResidentDiscard::Unknown).await
        } else {
            // A settled preparation can still include a failed baseline.
            // finish(fenced) avoids implicitly requesting resume via discard.
            self.rpc.finish(ResidentAdmission::Fenced).await
        };
        reply.is_ok_and(|reply| wire::settled(&self.binding, reply.into_unverified_value()))
    }
}

pub(super) async fn run(operation: Arc<Operation>, spec: Specification,
    baseline_turn: String, session: Session, control: PreparationControl)
    -> PreparationResult<Report> {
    let mut report = Report { cleanup_confirmed: true, ..Report::default() };
    if let Err(failure) = execute(&operation, spec, baseline_turn, &session, &control,
        &mut report).await { report.failure = Some(failure); }
    PreparationResult { cleanup_confirmed: report.cleanup_confirmed, value: Ok(report) }
}

fn active(control: &PreparationControl) -> Result<(), Failure> {
    if control.is_cancelled() { Err(Failure::Cancelled) } else { Ok(()) }
}
fn target(spec: &Specification) -> CredentialTarget {
    CredentialTarget { project: spec.identity.project.clone(), path: match spec.route {
        FreeRoute::Responses => "/ai/v3/responses/", FreeRoute::Messages => "/ai/v3/messages/",
    } }
}

async fn execute(operation: &Operation, spec: Specification, baseline_turn: String,
    session: &Session, control: &PreparationControl, report: &mut Report)
    -> Result<(), Failure> {
    active(control)?;
    let capture = &operation.capture;
    let page = &capture.page;
    let target = target(&spec);
    let plan = Plan::new(capture.scope.clone(), spec).map_err(|_| Failure::Context)?;
    let body = plan.comparison_body().map_err(|_| Failure::Context)?;
    let mut baseline_body = body.clone();
    baseline_body["turnId"] = Value::String(baseline_turn);
    let api = BoundControlClient::new(capture.scope.clone(), owner::egress(&page.effective_proxy))
        .map_err(|_| Failure::Context)?;
    let http = NativeHttp::new(capture.identity.account, capture.identity.user.clone(),
        NativeHttpLimits::default(), owner::egress(&page.effective_proxy))
        .map_err(|_| Failure::Context)?.with_resident_browser_headers();
    let budget = control.deadline().saturating_duration_since(Instant::now())
        .as_millis().min(u128::from(MAX_BUDGET_MS)) as u64;
    let request = ResidentAcquire::new(target.project.0.clone(), target.path.into(),
        page.wrapper_url.clone(), page.wrapper_export.clone(), session.cookie_header.clone(),
        budget).map_err(|_| Failure::Context)?;
    let identity = operation.binding.api().map_err(|_| Failure::Context)?;
    let started = std::time::Instant::now();
    let context = TrustedResidentContext::new(&capture.scope,
        operation.binding.api().map_err(|_| Failure::Context)?,
        CredentialTarget { project: target.project.clone(), path: target.path },
        page.page_url.clone(), page.device_id.clone(), owner::egress(&page.effective_proxy),
        started).map_err(|_| Failure::Context)?;
    active(control)?;
    operation.acquire_started.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| Failure::Acquire)?;
    let reply = operation.rpc.acquire(request).await.map_err(|_| {
        report.cleanup_confirmed = false; Failure::Acquire
    })?;
    let acquired = wire::acquired(&operation.binding, page, reply.into_unverified_value())?;
    active(control)?;
    let observation = ResidentObservation::from_unverified_reply(&identity, &target.project,
        target.path, &page.page_url, &page.device_id, &acquired.header,
        owner::egress(&page.effective_proxy));
    let grant = ResidentGrant::new_checked(&api, context, session, observation)
        .map_err(|_| Failure::Context)?;
    let bearer = api.resident_bearer(grant, control).await;
    report.cleanup_confirmed &= bearer.cleanup_confirmed;
    let bearer = bearer.value.map_err(|_| Failure::Bearer)?;
    if !report.cleanup_confirmed { return Err(Failure::Cleanup); }
    active(control)?;
    let baseline = capture.baseline.run(BaselineInput {
        identity: &capture.identity, page, lease: operation.binding.lease,
        configured_proxy_revision: capture.configured_proxy_revision,
        path: target.path, body: &baseline_body, cookie_header: &acquired.header,
        bearer: &bearer,
    }, control).await;
    report.cleanup_confirmed &= baseline.cleanup_confirmed;
    let baseline = baseline.value.map_err(|_| Failure::Baseline)?;
    if !report.cleanup_confirmed || baseline.configured_proxy_revision != capture.configured_proxy_revision
        || !wire::baseline_matches(&operation.binding,
        page, &capture.identity, target.path, &acquired.header, &baseline) {
        return Err(Failure::Baseline);
    }
    report.baseline_accepted = true;
    active(control)?;
    let observation = ResidentObservation::from_unverified_reply(&identity, &target.project,
        target.path, &baseline.page_url, &baseline.device_id, &baseline.cookie_header,
        owner::egress(&baseline.effective_proxy));
    let credentials = api.authorize_resident_token(bearer, session, observation,
        acquired.token, control).map_err(|_| Failure::Context)?;
    let current = Session { active_user: session.active_user.clone(),
        cookie_header: acquired.header };
    send(operation, &http, target.path, &body, &current, credentials, control, report).await
}

#[allow(clippy::too_many_arguments)]
async fn send(operation: &Operation, http: &NativeHttp, path: &str, body: &Value,
    current: &Session, credentials: PreparedCredentials, control: &PreparationControl,
    report: &mut Report) -> Result<(), Failure> {
    active(control)?;
    if credentials.current_cookies.is_none() { return Err(Failure::Context); }
    let account = operation.binding.account;
    let cancel = NativeCancellation::new();
    let material = credentials.material;
    let auth = AccountRequestAuth::new(account, current, &material.bearer);
    report.native_invoked = true; // Local invocation, not a measured socket send.
    let NativeTrackedRequest { response, closed } = http.ai_post_stream_tracked(
        &account, path, auth, body, material.castle, cancel.clone());
    let started = std::time::Instant::now();
    let result = {
        let work = async {
            let response = response.await.map_err(|_| Failure::Native)?;
            collect(response, started, report).await
        };
        tokio::pin!(work);
        tokio::select! {
            biased;
            _ = cancelled(control) => { cancel.cancel(); work.await }
            value = &mut work => value,
        }
    };
    report.cleanup_confirmed &= closed.await.is_ok();
    if !report.cleanup_confirmed { return Err(Failure::Cleanup); }
    active(control)?;
    result
}

async fn cancelled(control: &PreparationControl) {
    while !control.is_cancelled() {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

async fn collect(mut response: framer_api::experimental_native::native_http::NativeResponse,
    started: std::time::Instant, report: &mut Report) -> Result<(), Failure> {
    const LIMIT: usize = 64 * 1024;
    report.status = Some(response.status);
    let json = response.content_type.as_deref().is_some_and(|value|
        value.split(';').next().is_some_and(|media| media.trim() == "application/json"));
    let mut bytes = Vec::new();
    while let Some(chunk) = response.body.next().await {
        let chunk = chunk.map_err(|_| Failure::Native)?;
        if chunk.len() > LIMIT - bytes.len() { return Err(Failure::Native); }
        bytes.extend_from_slice(&chunk);
    }
    report.body_complete = true;
    report.json_object = json && serde_json::from_slice::<Value>(&bytes)
        .is_ok_and(|value| value.is_object());
    if response.status == 403 {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            report.refusal = Some(framer_api::classify::observe_403(text,
                started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64));
        }
    }
    if response.status == 200 && report.json_object { Ok(()) }
    else { Err(Failure::Native) }
}
