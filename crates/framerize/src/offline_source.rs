//! Inert source selection for the existing one-shot complete-JSON caller.
use std::sync::Arc;
use framer_api::experimental_native::{
    authorization::CredentialSource,
    control_authorization::{BoundControlClient, OfflineMintTarget, OfflineSessionSource},
    native_account::AccountScope,
    native_http::NativeEgress,
};
use super::{offline_worker::OfflineWorkerMinter, offline_worker_owner::Worker,
    one_shot_control::{Failure, SourceBuilder}};

pub(crate) struct OfflineSource {
    worker: Arc<Worker>,
    target: OfflineMintTarget,
}
impl OfflineSource {
    pub(crate) fn new(worker: impl Into<Arc<Worker>>, target: OfflineMintTarget) -> Self {
        Self { worker: worker.into(), target }
    }
}
impl SourceBuilder for OfflineSource {
    fn build(&self, scope: AccountScope, proxy: Option<String>)
        -> Result<Arc<dyn CredentialSource>, Failure> {
        let route = proxy.as_deref().map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct);
        let api = BoundControlClient::new(scope.clone(), route).map_err(|_| Failure::Source)?;
        let minter = OfflineWorkerMinter::new(scope, self.worker.clone());
        let source = OfflineSessionSource::new(Arc::new(api), self.target.clone(),
            Arc::new(minter)).map_err(|_| Failure::Source)?;
        Ok(Arc::new(source))
    }
}
