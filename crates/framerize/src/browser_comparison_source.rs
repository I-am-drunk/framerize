use super::*;
use framer_api::experimental_native::{authorization::{CredentialMaterial, CredentialTarget,
    PreparedCredentials}, native_account::{PreparationControl, PreparationFailure, PreparationResult}};
use framer_core::port::Session;
use futures::{future::BoxFuture, FutureExt};

pub(super) struct Source {
    pub api: Arc<dyn control::Control>,
    pub context: tokio::sync::Mutex<ComparisonContext>,
    pub scope: AccountScope,
    pub target: CredentialTarget,
}

impl CredentialSource for Source {
    fn issue<'a>(&'a self, _: &'a AccountScope, _: &'a Session,
        _: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        futures::future::ready(PreparationResult::finished(Err(PreparationFailure::Rejected))).boxed()
    }

    fn issue_for_target<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        target: &'a CredentialTarget, control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<PreparedCredentials>> {
        async move {
            if !self.scope.same_generation(scope) || target.project != self.target.project
                || target.path != self.target.path || control.is_cancelled() {
                return PreparationResult::finished(Err(PreparationFailure::Rejected));
            }
            let mut context = self.context.lock().await;
            if control.is_cancelled() {
                return PreparationResult::finished(Err(PreparationFailure::Cancelled));
            }
            let supplied = match context.native_token().await {
                Ok(value) => value, Err(_) => return PreparationResult::unconfirmed(),
            };
            self.api.authorize(scope, session, target, control, supplied).await
        }.boxed()
    }
}
