//! Explicit one-shot browser control for a future native-transport check.
//! Construction never enables a provider or selects a serving account.
use framer_api::experimental_native::{
    authorization::{CredentialMaterial, CredentialSource, CredentialTarget, PreparedCredentials},
    control_authorization::{BoundBrowserSessionSource, BoundControlClient},
    native_http::NativeEgress,
    native_account::{AccountScope, PreparationControl, PreparationFailure, PreparationResult},
    observed_authorization::{ObservedCredentialSource, ObservedSourceFactory,
        OpenedSource, OpenFailure, SourceCleanup, SourceControl, SourceIdentity},
};
use framer_core::port::{BrowserBridge, Session};
use framer_host::sidecar::observer::{ApplicationGeneration, CleanupExtent,
    CleanupOutcome, ObserverError, ObserverFactory, ScopeControl};
use std::{future::Future, pin::Pin, sync::Arc};

type Preparation<'a, T = CredentialMaterial> = Pin<Box<dyn Future<Output = PreparationResult<T>> + Send + 'a>>;

/// Supply the same explicit egress as the native HTTP scope. None selects
/// direct egress; no environment or account fallback is inferred here.
pub fn browser_control_source(observer: ObserverFactory,
    scope: AccountScope, proxy: Option<String>) -> Result<Arc<dyn CredentialSource>, String> {
    let egress = proxy.as_deref().map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct);
    let api = Arc::new(BoundControlClient::new(scope.clone(), egress)?);
    let factory = Arc::new(Factory { api, observer, expected: scope.clone(), proxy });
    Ok(Arc::new(ObservedCredentialSource::new(scope, factory)))
}

struct Factory {
    api: Arc<BoundControlClient>,
    observer: ObserverFactory,
    expected: AccountScope,
    proxy: Option<String>,
}

struct Control(ScopeControl);
impl SourceControl for Control {
    fn request_stop(&self) { self.0.request_stop(); }
    fn seal(&self) { self.0.seal_for_account_teardown(); }
}

impl ObservedSourceFactory for Factory {
    fn open(&self, scope: &AccountScope) -> Result<OpenedSource, OpenFailure> {
        if !self.expected.same_generation(scope) {
            return Err(OpenFailure::NotStarted);
        }
        let egress = self.proxy.as_deref().map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct);
        if !scope.matches_egress(&egress) { return Err(OpenFailure::NotStarted); }
        let identity = SourceIdentity::from_scope(scope);
        let opened = self.observer.one_shot_scope(identity.account,
            ApplicationGeneration(identity.generation)).map_err(|error| match error {
                // These are rejected before a supervisor or remote work starts.
                ObserverError::RuntimeUnavailable | ObserverError::Capacity
                    | ObserverError::AccountBusy => OpenFailure::NotStarted,
                // An existing uncertain owner must never be treated as idle.
                _ => OpenFailure::Unconfirmed,
            })?;
        let receipt_control = opened.control.clone();
        let cleanup = Box::pin(async move {
            match opened.cleanup.wait().await {
                CleanupOutcome::NotStarted { .. } => SourceCleanup::NotStarted,
                CleanupOutcome::Closed(receipt)
                    if receipt.account == identity.account
                        && receipt.application_generation == ApplicationGeneration(identity.generation)
                        && receipt.extent == CleanupExtent::AccountBrowserClosed
                        && receipt_control.matches_closed_receipt(&receipt) => {
                            SourceCleanup::Closed(identity)
                        }
                CleanupOutcome::Closed(_) | CleanupOutcome::Unconfirmed(_) => SourceCleanup::Unconfirmed,
            }
        });
        let source = Arc::new(ConfiguredSource {
            source: BoundBrowserSessionSource::new(self.api.clone(), opened.browser.clone()),
            browser: opened.browser,
            expected: scope.clone(),
            proxy: self.proxy.clone(),
        });
        Ok(OpenedSource { scope: scope.clone(), source,
            control: Arc::new(Control(opened.control)), cleanup })
    }
}

struct ConfiguredSource {
    source: BoundBrowserSessionSource,
    browser: Arc<dyn BrowserBridge>,
    expected: AccountScope,
    proxy: Option<String>,
}

impl CredentialSource for ConfiguredSource {
    fn issue_for_target<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        target: &'a CredentialTarget, control: &'a PreparationControl)
        -> Preparation<'a, PreparedCredentials> {
        Box::pin(async move {
            if !self.expected.same_generation(scope) || scope.active_user() != &session.active_user {
                return PreparationResult::finished(Err(PreparationFailure::Rejected));
            }
            if control.is_cancelled() {
                return PreparationResult::finished(Err(PreparationFailure::Cancelled));
            }
            if self.browser.set_proxy(&scope.account(), self.proxy.clone()).await.is_err() {
                return PreparationResult::unconfirmed();
            }
            self.source.issue_for_target(scope, session, target, control).await
        })
    }

    fn issue<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        control: &'a PreparationControl) -> Preparation<'a> {
        Box::pin(async move {
            if !self.expected.same_generation(scope) || scope.active_user() != &session.active_user {
                return PreparationResult::finished(Err(PreparationFailure::Rejected));
            }
            if control.is_cancelled() {
                return PreparationResult::finished(Err(PreparationFailure::Cancelled));
            }
            // The observed bridge permits this only before its first command.
            // It cannot infer or inherit another account's route.
            if self.browser.set_proxy(&scope.account(), self.proxy.clone()).await.is_err() {
                return PreparationResult::unconfirmed();
            }
            self.source.issue(scope, session, control).await
        })
    }
}
