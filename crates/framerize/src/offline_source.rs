//! Inert source selection for the existing one-shot complete-JSON caller.
use std::sync::Arc;
use framer_core::{native_device::{DeviceAuthority, DeviceFailure, DeviceGrant},
    port::{BrowserBridge, Session}};
use futures::{future::BoxFuture, FutureExt};
use framer_api::experimental_native::{
    authorization::{CredentialSource, CredentialMaterial, CredentialTarget,
        CurrentRequestCookies, PreparedCredentials},
    control_authorization::{BoundControlClient, OfflineMintTarget, OfflineSessionSource},
    native_account::{AccountScope, PreparationControl, PreparationFailure, PreparationResult},
    native_http::NativeEgress,
};
use super::{offline_worker::{OfflineWorkerMinter, V2SeedBinding, V2SeedTransfer, WorkerBackend},
    offline_worker_owner::{V2Worker, Worker},
    one_shot_control::{Failure, SourceBuilder}};

pub(crate) struct OfflineSource {
    worker: WorkerBackend,
    target: OfflineMintTarget,
    bridge: Arc<dyn BrowserBridge>,
}
impl OfflineSource {
    pub(crate) fn from_worker(worker: WorkerBackend, target: OfflineMintTarget,
        bridge: Arc<dyn BrowserBridge>) -> Self {
        Self { worker, target, bridge }
    }

    pub(crate) fn new(worker: impl Into<Arc<Worker>>, target: OfflineMintTarget,
        bridge: Arc<dyn BrowserBridge>) -> Self {
        Self { worker: WorkerBackend::V3(worker.into()), target, bridge }
    }

    pub(crate) fn new_v2(worker: impl Into<Arc<V2Worker>>, target: OfflineMintTarget,
        bridge: Arc<dyn BrowserBridge>) -> Self {
        Self { worker: WorkerBackend::V2(worker.into()), target, bridge }
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
        if matches!(&self.worker, WorkerBackend::V2(_))
            || device.authority == DeviceAuthority::Observation {
            return Ok(Arc::new(DeviceProofSource {
                transfer: V2SeedTransfer::new(scope.clone(), device.clone()),
                scope, worker: self.worker.clone(), bridge: self.bridge.clone(), grant: device,
                api: Arc::new(api), target: self.target.clone(),
            }));
        }
        let minter = OfflineWorkerMinter::with_backend(scope, self.worker.clone(),
            self.bridge.clone(), device).map_err(|_| Failure::MissingAuthority)?;
        let source = OfflineSessionSource::new(Arc::new(api), self.target.clone(),
            Arc::new(minter)).map_err(|_| Failure::Source)?;
        Ok(Arc::new(source))
    }
}

struct DeviceProofSource {
    scope: AccountScope,
    worker: WorkerBackend,
    bridge: Arc<dyn BrowserBridge>,
    grant: Arc<DeviceGrant>,
    transfer: V2SeedTransfer,
    api: Arc<BoundControlClient>,
    target: OfflineMintTarget,
}

impl DeviceProofSource {
    async fn issue_bound(&self, scope: &AccountScope, session: &Session,
        target: &CredentialTarget, control: &PreparationControl)
        -> PreparationResult<PreparedCredentials> {
        if !self.scope.same_generation(scope) || scope.active_user() != &session.active_user
            || self.target.project() != &target.project || self.target.path() != target.path
            || CurrentRequestCookies::from_current_jar(session, session.cookie_header.clone()).is_err() {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        let proof = match self.transfer.get(scope, session, self.bridge.as_ref()).await {
            Ok(proof) => proof,
            Err(DeviceFailure::Rejected) =>
                return PreparationResult::finished(Err(PreparationFailure::Rejected)),
            Err(DeviceFailure::Unconfirmed) => return PreparationResult::unconfirmed(),
        };
        let seed = match V2SeedBinding::from_proof(&self.scope, self.grant.clone(), session, proof) {
            Ok(seed) => seed,
            Err(_) => return PreparationResult::finished(Err(PreparationFailure::Rejected)),
        };
        let minter = match OfflineWorkerMinter::with_host_proof(self.scope.clone(), self.worker.clone(),
            self.bridge.clone(), self.grant.clone(), seed) {
            Ok(minter) => minter,
            Err(_) => return PreparationResult::finished(Err(PreparationFailure::Rejected)),
        };
        let Ok(source) = OfflineSessionSource::new(self.api.clone(), self.target.clone(), Arc::new(minter))
            else { return PreparationResult::finished(Err(PreparationFailure::Rejected)); };
        source.issue_for_target(scope, session, target, control).await
    }
}

impl CredentialSource for DeviceProofSource {
    fn issue<'a>(&'a self, _: &'a AccountScope, _: &'a Session,
        _: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async { PreparationResult::finished(Err(PreparationFailure::Rejected)) }.boxed()
    }

    fn issue_for_target<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        target: &'a CredentialTarget, control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<PreparedCredentials>> {
        self.issue_bound(scope, session, target, control).boxed()
    }
}
