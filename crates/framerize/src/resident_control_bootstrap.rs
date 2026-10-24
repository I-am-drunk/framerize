//! Startup-owned adapter. Never installs itself into an existing process.
use super::*;
use super::owner::{CaptureError, egress};
use framer_api::experimental_native::session_store::{OpenAccountError, SessionStoreBridge};
use framer_core::port::SessionStore;
use framer_host::sidecar::{SidecarBridge, resident_bootstrap::BootstrapError};

/// install() replaces the application's shared session boundary. Retain this
/// owner and construct once, before admission; never per request.
pub(crate) struct StartupOwner {
    sidecar: Arc<SidecarBridge>,
    native: NativeAccountClient,
    store: Arc<SessionStoreBridge>,
    sdk: Option<Arc<offline_worker_owner::V2Worker>>,
}

impl StartupOwner {
    fn new(sidecar: Arc<SidecarBridge>, store: Arc<dyn SessionStore>)
        -> Result<Self, Failure> {
        let native = NativeAccountClient::new().map_err(|_| Failure::MissingOwner)?;
        let store = SessionStoreBridge::new(native.clone(), store)
            .map_err(|_| Failure::MissingOwner)?;
        Ok(Self { sidecar, native, store: Arc::new(store), sdk: None })
    }

    pub fn install(state: &mut AppState, sidecar: Arc<SidecarBridge>)
        -> Result<Arc<Self>, Failure> {
        Self::install_with_sdk(state, sidecar, None)
    }

    pub fn install_with_sdk(state: &mut AppState, sidecar: Arc<SidecarBridge>,
        sdk: Option<Arc<offline_worker_owner::V2Worker>>) -> Result<Arc<Self>, Failure> {
        if state.native_dispatch.is_some() || state.operation_ownership.is_none() {
            return Err(Failure::MissingOwner);
        }
        let mut owner = Self::new(sidecar, state.sessions.clone())?;
        owner.sdk = sdk;
        let owner = Arc::new(owner);
        state.sessions = owner.store.clone();
        Ok(owner)
    }

    pub async fn shutdown(&self, state: &AppState) -> Result<(), &'static str> {
        let shared: Arc<dyn SessionStore> = self.store.clone();
        if !Arc::ptr_eq(&shared, &state.sessions) { return Err("different resident session owner"); }
        let owner = state.operation_ownership.as_ref().ok_or("missing resident owner")?;
        let mut seal = owner.seal().map_err(|_| "resident shutdown admission failed")?;
        let drained = seal.wait_application().await.map_err(|_| "resident work unconfirmed")?;
        owner.check_application_drained(&drained).map_err(|_| "resident work unconfirmed")?;
        self.store.shutdown().await?;
        self.native.shutdown_idle().await.map_err(|_| "resident native shutdown unconfirmed")
    }
}

fn error(failure: Failure, cleanup_confirmed: bool) -> CaptureError {
    CaptureError { failure, cleanup_confirmed }
}

impl Owner for StartupOwner {
    fn capture<'a>(&'a self, identity: &'a Identity,
        configured_proxy: &'a Option<String>, path: &'static str, deadline: Instant)
        -> BoxFuture<'a, Result<Capture, CaptureError>> {
        async move {
            if Instant::now() >= deadline { return Err(error(Failure::Deadline, true)); }
            let metadata = self.sidecar.resident_bootstrap(identity.account,
                &identity.project.0, path, configured_proxy).await.map_err(|failure| {
                    error(Failure::Context, !matches!(failure, BootstrapError::Unconfirmed))
                })?;
            // A successful bootstrap has joined its read-only work and lease.
            if Instant::now() >= deadline { return Err(error(Failure::Deadline, true)); }
            let scope = self.store.open_account_observed(identity.account, identity.user.clone(),
                NativeHttpLimits::default(), egress(&metadata.effective_proxy)).await
                .map_err(|failure| error(Failure::Context,
                    matches!(failure, OpenAccountError::Rejected(_))))?;
            if Instant::now() >= deadline {
                let settled = match self.native.close_account(&scope) {
                    Ok(close) => close.await.is_ok(), Err(_) => false,
                };
                return Err(error(Failure::Deadline, settled));
            }
            Ok(Capture { identity: identity.clone(), configured_proxy: configured_proxy.clone(),
                configured_proxy_revision: metadata.proxy_revision,
                sidecar: self.sidecar.clone(), native: self.native.clone(), scope,
                sdk: self.sdk.clone(),
                page: CapturedPage {
                    sidecar_incarnation: metadata.sidecar_incarnation,
                    provenance: metadata.provenance,
                    application_generation: metadata.application_generation.0,
                    host_generation: metadata.host_generation,
                    account_incarnation: metadata.account_incarnation,
                    page_url: metadata.page_url, device_id: metadata.device,
                    effective_proxy: metadata.effective_proxy,
                    wrapper_url: metadata.wrapper_url, wrapper_export: metadata.wrapper_export,
                } })
        }.boxed()
    }
}

#[cfg(test)]
#[path = "resident_control_bootstrap_tests.rs"]
mod tests;
