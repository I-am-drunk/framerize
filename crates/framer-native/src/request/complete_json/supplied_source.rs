use super::*;
use crate::authorization::BoundAttemptContext;
use crate::authorization::CredentialSource;
use crate::authorization::GenerationAuthorization;
use crate::account::client::NativeAccountClient;

impl Plan {
    fn source_authorization(&self, native: NativeAccountClient,
        source: Arc<dyn CredentialSource>)
        -> Result<Arc<dyn AttemptAuthorization>, port::DispatchError> {
        let binding = |index: usize| BoundAttemptContext {
            project: self.contract.identity.project.clone(),
            chat_session: self.contract.identity.chat_session.clone(), step_index: index as u32,
            context: self.contract.context_at(index).expect("bounded context index").clone(),
        };
        let authorization = if self.contract.continuation.is_some() {
            GenerationAuthorization::for_sequence(native, self.contract.scope.clone(), source,
                (0..self.contract.max_attempts()).map(binding).collect())
        } else {
            GenerationAuthorization::for_attempt(native, self.contract.scope.clone(), source, binding(0))
        }.map_err(|_| invalid())?;
        Ok(Arc::new(authorization))
    }

    pub async fn launch_with_source_in_pool(self, native: NativeAccountClient,
        source: Arc<dyn CredentialSource>, permit: OwnedSemaphorePermit,
        admission: port::PoolAdmission) -> Result<ProbeRun, port::DispatchError> {
        let authorization = self.source_authorization(native.clone(), source)?;
        self.launch_in_pool(native, authorization, permit, admission).await
    }
}
