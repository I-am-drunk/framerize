//! Inert source selection for the existing one-shot complete-JSON caller.
use std::sync::Arc;
use framer_core::{native_device::DeviceGrant, port::BrowserBridge};
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
    bridge: Arc<dyn BrowserBridge>,
}
impl OfflineSource {
    pub(crate) fn new(worker: impl Into<Arc<Worker>>, target: OfflineMintTarget,
        bridge: Arc<dyn BrowserBridge>) -> Self {
        Self { worker: worker.into(), target, bridge }
    }
}
impl SourceBuilder for OfflineSource {
    fn build(&self, _scope: AccountScope, _proxy: Option<String>)
        -> Result<Arc<dyn CredentialSource>, Failure> {
        Err(Failure::MissingAuthority)
    }

    fn device_bridge(&self) -> Option<Arc<dyn BrowserBridge>> { Some(self.bridge.clone()) }

    fn build_with_device(&self, scope: AccountScope, proxy: Option<String>,
        device: Option<Arc<DeviceGrant>>)
        -> Result<Arc<dyn CredentialSource>, Failure> {
        let device = device.ok_or(Failure::MissingAuthority)?;
        if device.account != scope.account() { return Err(Failure::MissingAuthority); }
        let route = proxy.as_deref().map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct);
        let api = BoundControlClient::new(scope.clone(), route).map_err(|_| Failure::Source)?;
        let minter = OfflineWorkerMinter::with_device(scope, self.worker.clone(),
            self.bridge.clone(), device).map_err(|_| Failure::MissingAuthority)?;
        let source = OfflineSessionSource::new(Arc::new(api), self.target.clone(),
            Arc::new(minter)).map_err(|_| Failure::Source)?;
        Ok(Arc::new(source))
    }
}
