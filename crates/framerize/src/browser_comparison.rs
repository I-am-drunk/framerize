//! Explicit two-request comparison; never selected by CredentialSource.
//! The outer controller owns admission, cooldown publication and process exit.
use super::*;
use framer_api::experimental_native::control_authorization::BoundControlClient;
use framer_host::sidecar::observer::{ApplicationGeneration, CleanupOutcome, ObserverFactory};
use framer_host::sidecar::observer::comparison::{BaselineReport, ComparisonClient,
    ComparisonContext, ComparisonPlan};
use std::sync::Mutex;

#[path = "browser_comparison_attempt.rs"]
mod attempt;
#[path = "browser_comparison_source.rs"]
mod source;

pub(crate) trait ComparisonWindow: Send + Sync {
    /// Exclude conflicting work and admit this bounded pair before private I/O.
    fn admit(&self) -> FutureResult<'_, Result<Selection, Failure>>;
    /// Preserve both results and cooldowns. This never releases exclusion.
    fn record(&self, report: &Report) -> FutureResult<'_, Result<(), Failure>>;
}

#[derive(Default)]
pub(crate) struct Report {
    pub baseline: Option<BaselineReport>,
    pub native: Option<LocalReport>,
    pub native_nonempty_text: bool,
    pub browser_cleanup_confirmed: bool,
    pub account_closed: bool,
    pub failure: Option<Failure>,
}

pub(crate) struct Dependencies {
    pub authority: Arc<dyn ComparisonWindow>,
    pub store: Arc<dyn SessionStore>,
    pub observer: ObserverFactory,
    pub permit: OwnedSemaphorePermit,
}

pub(crate) fn launch(spec: Specification, baseline_turn: String, deps: Dependencies)
    -> Result<FutureResult<'static, Report>, Failure> {
    Plan::validate(&spec).map_err(|_| Failure::Context)?;
    if baseline_turn.is_empty() || baseline_turn.len() > 256
        || baseline_turn.chars().any(char::is_control)
        || baseline_turn == spec.context.turn_id {
        return Err(Failure::Context);
    }
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| Failure::OwnerLost)?;
    let (sender, receiver) = oneshot::channel();
    drop(runtime.spawn(async move {
        let report = run(spec, baseline_turn, deps).await;
        let _ = sender.send(report);
    }));
    Ok(Box::pin(async move { receiver.await.unwrap_or_else(|_| Report {
        failure: Some(Failure::OwnerLost), ..Report::default()
    }) }))
}

async fn run(spec: Specification, baseline_turn: String, deps: Dependencies) -> Report {
    let authority = deps.authority.clone();
    let mut report = match authority.admit().await {
        Ok(selected) if selected.identity == spec.identity
            && same_context(&selected.context, &spec.context) => {
                owned(spec, baseline_turn, selected, deps).await
            }
        Ok(_) => Report { failure: Some(Failure::Context), ..Report::default() },
        Err(error) => return Report { failure: Some(error), ..Report::default() },
    };
    if authority.record(&report).await.is_err() {
        report.failure = Some(Failure::CooldownPublication);
    }
    report
}

async fn owned(spec: Specification, baseline_turn: String, selected: Selection,
    deps: Dependencies) -> Report {
    let mut report = Report::default();
    let Ok(native) = NativeAccountClient::new() else {
        report.failure = Some(Failure::Session); return report;
    };
    let Ok(store) = SessionStoreBridge::new(native.clone(), deps.store) else {
        report.failure = Some(Failure::Session); return report;
    };
    let egress = selected.proxy.as_deref().map(NativeEgress::Proxy)
        .unwrap_or(NativeEgress::Direct);
    match store.open_account(selected.identity.account, selected.identity.user.clone(),
        NativeHttpLimits::default(), egress).await {
        Ok(scope) => {
            report = execute(spec, baseline_turn, selected, &native, &scope,
                deps.observer, deps.permit).await;
        }
        Err(_) => report.failure = Some(Failure::Session),
    }
    if store.shutdown().await.is_err() {
        report.account_closed = false;
        report.failure = Some(Failure::Cleanup);
    }
    report
}

async fn execute(spec: Specification, baseline_turn: String, selected: Selection,
    native: &NativeAccountClient, scope: &AccountScope, observer: ObserverFactory,
    permit: OwnedSemaphorePermit) -> Report {
    let saved = Arc::new(Mutex::new(Report::default()));
    let opened = observer.comparison_scope(scope.account(), ApplicationGeneration(scope.generation()));
    let mut cleanup = None;
    match opened {
        Ok(opened) => {
            cleanup = Some((opened.control, opened.cleanup));
            let task_native = native.clone();
            let task_scope = scope.clone();
            let task_report = saved.clone();
            let result = tokio::spawn(async move {
                attempt::run(spec, baseline_turn, selected, task_native, task_scope,
                    opened.client, permit, task_report).await
            }).await;
            saved.lock().unwrap().failure = match result {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error),
                Err(_) => Some(Failure::OwnerLost),
            };
        }
        Err(_) => saved.lock().unwrap().failure = Some(Failure::Source),
    }
    let account_closed = match native.close_account(scope) {
        Ok(closed) => closed.await.is_ok(), Err(_) => false,
    };
    let mut report = std::mem::take(&mut *saved.lock().unwrap());
    report.account_closed = account_closed;
    if !account_closed { report.failure = Some(Failure::Cleanup); }
    if let Some((control, cleanup)) = cleanup {
        if report.failure.is_some() { control.request_stop(); }
        else { control.seal_for_account_teardown(); }
        report.browser_cleanup_confirmed = match cleanup.wait().await {
            CleanupOutcome::NotStarted { .. } => true,
            CleanupOutcome::Closed(receipt) => control.matches_closed_receipt(&receipt),
            CleanupOutcome::Unconfirmed(_) => false,
        };
        if !report.browser_cleanup_confirmed { report.failure = Some(Failure::Cleanup); }
    }
    report
}
