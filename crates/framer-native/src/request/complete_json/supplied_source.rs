use super::*;
use crate::authorization::BoundAttemptContext;
use crate::authorization::CredentialSource;
use crate::authorization::GenerationAuthorization;
use crate::account::client::NativeAccountClient;

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

    pub async fn launch_with_source_in_pool(self, native: NativeAccountClient,
        source: Arc<dyn CredentialSource>, permit: OwnedSemaphorePermit,
        admission: port::PoolAdmission) -> Result<ProbeRun, port::DispatchError> {
        let authorization = self.source_authorization(native.clone(), source)?;
        self.launch_in_pool(native, authorization, permit, admission).await
    }
}
