use super::*;
use framer_api::experimental_native::{authorization::CredentialTarget,
    native_account::{PreparationFailure, PreparationResult}};
use framer_api::native_dispatch::complete_json::{CompletedJson, FreeRoute};

pub(super) async fn run(spec: Specification, baseline_turn: String, selected: Selection,
    native: NativeAccountClient, scope: AccountScope, client: ComparisonClient,
    permit: OwnedSemaphorePermit, saved: Arc<Mutex<Report>>) -> Result<(), Failure> {
    let deadline = spec.deadline;
    let target = CredentialTarget { project: spec.identity.project.clone(), path: match spec.route {
        FreeRoute::Responses => "/ai/v3/responses/", FreeRoute::Messages => "/ai/v3/messages/",
    } };
    let plan = Plan::new(scope.clone(), spec).map_err(|_| Failure::Context)?;
    let native_body = plan.comparison_body().map_err(|_| Failure::Context)?;
    let mut baseline_body = native_body.clone();
    baseline_body["turnId"] = serde_json::Value::String(baseline_turn);
    let comparison = ComparisonPlan::new(target.project.clone(), target.path,
        baseline_body, native_body).map_err(|_| Failure::Context)?;
    let egress = selected.proxy.as_deref().map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct);
    let api = Arc::new(BoundControlClient::new(scope.clone(), egress).map_err(|_| Failure::Source)?);
    client.set_proxy(selected.proxy).await.map_err(|_| Failure::Source)?;
    let leases = AccountLeaseCoordinator::new(LeaseLimits { accounts: 1, users: 1,
        waiters: 2, user_id_bytes: 256, total_user_bytes: 256 }).map_err(|_| Failure::Bootstrap)?;
    bootstrap(&leases, &selected.identity).await?;
    let context = baseline(&native, &scope, client, comparison, api.clone(), &target,
        deadline, saved.clone()).await?;
    if !saved.lock().unwrap().baseline.as_ref().is_some_and(|b| b.accepted) {
        return Err(Failure::Dispatch);
    }
    let source = Arc::new(source::Source { api, context: tokio::sync::Mutex::new(context),
        scope: scope.clone(), target });
    let run = plan.launch_with_source(Arc::new(HttpFramerClient::new()), native,
        source, leases, permit).await.map_err(|_| Failure::Dispatch)?;
    let evidence = run.result.await;
    let nonempty = nonempty(run.completed.await);
    let accepted = evidence.accepted && nonempty;
    let mut report = saved.lock().unwrap();
    report.native_nonempty_text = nonempty;
    report.native = Some(LocalReport::from_evidence(evidence));
    if accepted { Ok(()) } else { Err(Failure::Dispatch) }
}

async fn baseline(native: &NativeAccountClient, scope: &AccountScope, client: ComparisonClient,
    plan: ComparisonPlan, api: Arc<BoundControlClient>, target: &CredentialTarget,
    deadline: tokio::time::Instant, saved: Arc<Mutex<Report>>) -> Result<ComparisonContext, Failure> {
    let target = CredentialTarget { project: target.project.clone(), path: target.path };
    let expected = scope.clone();
    let pending = native.prepare_owned(scope, deadline, move |session, control| async move {
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        let mut context = match client.prepare(plan, &session.cookie_header).await {
            Ok(value) => value, Err(_) => return PreparationResult::unconfirmed(),
        };
        let header = context.cookie_header().to_owned();
        let bearer = api.browser_context_bearer(&expected, &session, &target,
            context.page_url(), &header, &control).await;
        let bearer = match bearer.value {
            Ok(value) => value,
            Err(error) => return PreparationResult { value: Err(error),
                cleanup_confirmed: bearer.cleanup_confirmed },
        };
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        let report = match context.baseline(&bearer, &header).await {
            Ok(value) => value, Err(_) => return PreparationResult::unconfirmed(),
        };
        let settled = report.fetch_settled;
        saved.lock().unwrap().baseline = Some(report);
        if !settled { return PreparationResult::unconfirmed(); }
        PreparationResult::finished(Ok(context))
    }).map_err(|_| Failure::Admission)?;
    pending.await.map_err(|_| Failure::Source)
}

fn nonempty(completed: Option<CompletedJson>) -> bool {
    fn text(block: &serde_json::Value, kind: &str) -> bool {
        block["type"] == kind && block["text"].as_str().is_some_and(|s| !s.is_empty())
    }
    match completed {
        Some(CompletedJson::Responses(value)) => value.output.iter().any(|item|
            item["content"].as_array().is_some_and(|blocks|
                blocks.iter().any(|block| text(block, "output_text")))),
        Some(CompletedJson::Messages(value)) => value.content.iter().any(|block| text(block, "text")),
        None => false,
    }
}
