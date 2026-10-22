//! Unregistered caller. Global exclusion and final process acceptance belong
//! to the outer controller, not to this local receipt or an account lease.
use framer_api::{HttpFramerClient, experimental_native::{
    application::AttemptContext, authorization::CredentialSource,
    native_account::{AccountScope, NativeAccountClient},
    native_http::{NativeEgress, NativeHttpLimits}, session_store::SessionStoreBridge,
}, native_dispatch::complete_json::{Evidence, Plan, RefusalObservation, Specification}};
use framer_core::{account_lease::{AccountLeaseCoordinator, ChangeResult, LeaseLimits},
    native_dispatch::{Identity, FailureKind}, port::SessionStore};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::{oneshot, OwnedSemaphorePermit};

type FutureResult<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    MissingAuthority, Cooling, WavePaused, Admission, Context, Session,
    Bootstrap, Source, Dispatch, Cleanup, CooldownPublication, OwnerLost,
}

/// Implement only in the outer standalone/pre-serving controller. Admission
/// must admit at most one control, atomically exclude ALL old operations/admin/session writers, check
/// saved cooldown/WaveGuard, then read current identity/context/explicit route.
/// Dropping this Rust handle MUST NOT release the external exclusion window.
/// The controller keeps its independent hold until process descendants exit.
/// No production implementation exists in this source slice.
pub(crate) trait ExclusiveWindow: Send + Sync {
    fn admit(&self) -> FutureResult<'_, Result<Selection, Failure>>;
    /// Apply existing cooldown policy under the same exclusion, using bounded
    /// facts, never a stale account clone. No clearing existing cooldowns.
    /// Authorization failures without an HTTP status are unclassified. Keep
    /// exclusion held for explicit reconciliation; never invent a cooldown or
    /// infer an upstream refusal from missing status. Record is NOT release.
    fn record(&self, report: &LocalReport) -> FutureResult<'_, Result<(), Failure>>;
}

pub(crate) struct Selection {
    pub identity: Identity,
    pub context: AttemptContext,
    pub proxy: Option<String>,
}

/// Construction only: build must not start external work. All issuance and
/// cleanup must use CredentialSource ownership. The production implementation
/// delegates to the existing observed Factory; tests inject offline sources.
pub(crate) trait SourceBuilder: Send + Sync {
    fn build(&self, scope: AccountScope, proxy: Option<String>)
        -> Result<Arc<dyn CredentialSource>, Failure>;

    /// The caller retains the host grant through the whole request cleanup.
    /// Merely querying this capability starts no external work.
    fn device_bridge(&self) -> Option<Arc<dyn framer_core::port::BrowserBridge>> { None }

    fn build_with_device(&self, scope: AccountScope, proxy: Option<String>,
        device: Option<Arc<framer_core::native_device::DeviceGrant>>)
        -> Result<Arc<dyn CredentialSource>, Failure> {
        if device.is_some() { return Err(Failure::MissingAuthority); }
        self.build(scope, proxy)
    }
}
pub(crate) struct BrowserSource(pub framer_host::sidecar::observer::ObserverFactory);
impl SourceBuilder for BrowserSource {
    fn build(&self, scope: AccountScope, proxy: Option<String>)
        -> Result<Arc<dyn CredentialSource>, Failure> {
        crate::observed_authorization::browser_control_source(self.0.clone(), scope, proxy)
            .map_err(|_| Failure::Source)
    }
}

/// Intentionally omits models/text/raw errors/headers/credentials and any
/// final acceptance bit. Local success never proves descendant teardown.
#[derive(Clone, Debug)]
pub(crate) struct LocalReport {
    pub complete_json: bool,
    pub status: Option<u16>,
    pub attempts: usize,
    pub authorization_calls: usize,
    pub transport_starts: usize,
    pub local_cleanup: bool,
    pub failure: Option<Failure>,
    pub dispatch_failure: Option<FailureKind>,
    pub refusal: Option<RefusalObservation>,
}
impl LocalReport {
    fn failed(failure: Failure) -> Self {
        Self { complete_json: false, status: None, attempts: 0,
            authorization_calls: 0, transport_starts: 0,
            local_cleanup: false, failure: Some(failure), dispatch_failure: None,
            refusal: None }
    }
    fn from_evidence(e: Evidence) -> Self {
        Self { complete_json: e.complete_json, status: e.status, attempts: e.attempts,
            authorization_calls: e.authorization_calls, transport_starts: e.transport_starts,
            local_cleanup: e.cleanup_confirmed, dispatch_failure: e.failure,
            refusal: e.refusal,
            failure: if e.accepted { None } else { Some(Failure::Dispatch) } }
    }
}

pub(crate) struct Dependencies {
    pub authority: Option<Arc<dyn ExclusiveWindow>>,
    pub store: Arc<dyn SessionStore>,
    pub source: Arc<dyn SourceBuilder>,
    pub permit: OwnedSemaphorePermit,
}

/// Synchronous launch transfers all cleanup ownership before returning a waiter.
/// The external supervisor must bound this task's process lifetime; timing out
/// cleanup locally would not justify releasing exclusion or reporting success.
pub(crate) fn launch(spec: Specification, deps: Dependencies)
    -> Result<FutureResult<'static, LocalReport>, Failure> {
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| Failure::OwnerLost)?;
    let (sender, receiver) = oneshot::channel();
    let owner = async move {
        let report = run(spec, deps).await;
        let _ = sender.send(report);
    };
    // Detached on purpose: dropping the result cannot abort private-store jobs,
    // browser teardown, account closure or cooldown publication.
    drop(runtime.spawn(owner));
    Ok(Box::pin(async move {
        receiver.await.unwrap_or_else(|_| LocalReport::failed(Failure::OwnerLost))
    }))
}

fn same_context(a: &AttemptContext, b: &AttemptContext) -> bool {
    a.feature_category == b.feature_category && a.feature_name == b.feature_name
        && a.step == b.step && a.turn_id == b.turn_id && a.user_prompt == b.user_prompt
}

async fn bootstrap(leases: &AccountLeaseCoordinator, identity: &Identity)
    -> Result<(), Failure> {
    let mut change = leases.begin_change(identity.account).map_err(|_| Failure::Bootstrap)?
        .await.map_err(|_| Failure::Bootstrap)?;
    change.fence_user(&identity.user.0).map_err(|_| Failure::Bootstrap)?
        .await.map_err(|_| Failure::Bootstrap)?;
    change.stage_binding(identity.account, Some(&identity.user.0))
        .map_err(|_| Failure::Bootstrap)?;
    change.finish(ChangeResult::Completed).map_err(|_| Failure::Bootstrap)
}

async fn run(spec: Specification, deps: Dependencies) -> LocalReport {
    let Dependencies { authority, store, source, permit } = deps;
    let Some(authority) = authority else { return LocalReport::failed(Failure::MissingAuthority); };
    let selected = match authority.admit().await {
        Ok(value) => value, Err(error) => return LocalReport::failed(error),
    };
    let mut report = if selected.identity != spec.identity
        || !same_context(&selected.context, &spec.context) {
        LocalReport::failed(Failure::Context)
    } else {
        owned(spec, selected, store, source, permit).await
    };
    if authority.record(&report).await.is_err() {
        report.failure = Some(Failure::CooldownPublication);
    }
    report
}

async fn owned(spec: Specification, selected: Selection, store: Arc<dyn SessionStore>,
    source: Arc<dyn SourceBuilder>, permit: OwnedSemaphorePermit) -> LocalReport {
    let native = match NativeAccountClient::new() {
        Ok(value) => value, Err(_) => return LocalReport::failed(Failure::Session),
    };
    let bridge = match SessionStoreBridge::new(native.clone(), store) {
        Ok(value) => value, Err(_) => return LocalReport::failed(Failure::Session),
    };
    let egress = selected.proxy.as_deref().map(NativeEgress::Proxy)
        .unwrap_or(NativeEgress::Direct);
    let opened = bridge.open_account(selected.identity.account,
        selected.identity.user.clone(), NativeHttpLimits::default(), egress).await;
    let mut report = match opened {
        Ok(scope) => execute(spec, selected, source, permit, &native, &scope).await,
        Err(_) => LocalReport::failed(Failure::Session),
    };
    if bridge.shutdown().await.is_err() {
        report.local_cleanup = false;
        report.failure = Some(Failure::Cleanup);
    }
    report
}

async fn execute(spec: Specification, selected: Selection, source: Arc<dyn SourceBuilder>,
    permit: OwnedSemaphorePermit, native: &NativeAccountClient, scope: &AccountScope)
    -> LocalReport {
    let task_native = native.clone();
    let task_scope = scope.clone();
    // A source-builder panic still leaves this owner holding the account close
    // observer. No abort handle is exposed to the result waiter.
    let task = tokio::spawn(async move {
        attempt(spec, selected, source, permit, task_native, task_scope).await
    });
    let (mut report, no_invocation) = match task.await {
        Ok(Ok(value)) => (LocalReport::from_evidence(value), false),
        Ok(Err(error)) => {
            let mut report = LocalReport::failed(error.failure);
            report.dispatch_failure = error.dispatch_failure;
            (report, true)
        },
        Err(_) => (LocalReport::failed(Failure::OwnerLost), false),
    };
    let closed = match native.close_account(scope) {
        Ok(observer) => observer.await.is_ok(), Err(_) => false,
    };
    if !closed {
        report.local_cleanup = false;
        report.failure = Some(Failure::Cleanup);
    } else if no_invocation {
        report.local_cleanup = true;
    }
    report
}

struct AttemptFailure { failure: Failure, dispatch_failure: Option<FailureKind> }
impl From<Failure> for AttemptFailure {
    fn from(failure: Failure) -> Self { Self { failure, dispatch_failure: None } }
}

async fn attempt(spec: Specification, selected: Selection, source: Arc<dyn SourceBuilder>,
    permit: OwnedSemaphorePermit, native: NativeAccountClient, scope: AccountScope)
    -> Result<Evidence, AttemptFailure> {
    let plan = Plan::new(scope.clone(), spec).map_err(|_| Failure::Context)?;
    let leases = AccountLeaseCoordinator::new(LeaseLimits { accounts: 1, users: 1,
        waiters: 2, user_id_bytes: 256, total_user_bytes: 256 })
        .map_err(|_| Failure::Bootstrap)?;
    bootstrap(&leases, &selected.identity).await?;
    let source = source.build(scope, selected.proxy)?;
    let run = plan.launch_with_source(Arc::new(HttpFramerClient::new()), native,
        source, leases, permit).await.map_err(|error| AttemptFailure {
            failure: Failure::Dispatch, dispatch_failure: Some(error.kind),
        })?;
    Ok(run.result.await)
}

#[cfg(test)]
#[path = "one_shot_control_tests.rs"]
mod tests;
