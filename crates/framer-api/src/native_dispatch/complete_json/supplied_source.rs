use super::*;
use crate::{HttpFramerClient, experimental_native::{
    authorization::{BoundAttemptContext, CredentialSource, GenerationAuthorization},
    native_account::NativeAccountClient,
}};
use framer_core::account_lease::AccountLeaseCoordinator;

impl Plan {
    fn source_authorization(&self, native: NativeAccountClient,
        source: Arc<dyn CredentialSource>)
        -> Result<Arc<dyn AttemptAuthorization>, port::DispatchError> {
        let context = &self.contract.context;
        let authorization = GenerationAuthorization::for_attempt(native.clone(),
            self.contract.scope.clone(), source, BoundAttemptContext {
                project: self.contract.identity.project.clone(),
                chat_session: self.contract.identity.chat_session.clone(), step_index: 0,
                context: AttemptContext {
                    feature_category: context.feature_category.clone(),
                    feature_name: context.feature_name.clone(), step: context.step.clone(),
                    turn_id: context.turn_id.clone(), user_prompt: context.user_prompt.clone(),
                },
            }).map_err(|_| invalid())?;
        Ok(Arc::new(authorization))
    }

    /// Use the coordinator shared with legacy work and lifecycle changes.
    pub async fn launch_with_source(self, api: Arc<HttpFramerClient>,
        native: NativeAccountClient, source: Arc<dyn CredentialSource>,
        leases: AccountLeaseCoordinator, permit: OwnedSemaphorePermit)
        -> Result<ProbeRun, port::DispatchError> {
        let authorization = self.source_authorization(native.clone(), source)?;
        self.launch(authorization, permit, move |binding, policy| {
            let provider = crate::native_dispatch::NativeDispatchProvider::new(
                api, native, vec![binding], policy)?.with_account_leases(leases);
            Ok(Arc::new(provider) as Arc<dyn port::InvocationProvider>)
        }).await
    }

    /// Preserve the live pool's admission through preparation and cleanup.
    /// No separate account coordinator or replacement admission is created.
    pub async fn launch_with_source_in_pool(self, api: Arc<HttpFramerClient>,
        native: NativeAccountClient, source: Arc<dyn CredentialSource>,
        permit: OwnedSemaphorePermit, admission: port::PoolAdmission)
        -> Result<ProbeRun, port::DispatchError> {
        let authorization = self.source_authorization(native.clone(), source)?;
        self.launch_in_pool(authorization, permit, admission, move |binding, policy| {
            Ok(Arc::new(crate::native_dispatch::NativeDispatchProvider::new(
                api, native, vec![binding], policy)?) as Arc<dyn port::InvocationProvider>)
        }).await
    }
}
