//! Private ownership contract used by the optional startup adapter.
use super::*;
use framer_host::sidecar::SidecarBridge;

/// The installed application owner must return already-owned, idle resources.
/// Capture may await private bootstrap metadata and an exclusive native scope;
/// it must not get-or-create a browser, navigate, mint or issue inference.
/// The native scope is transferred exclusively and will be closed.
/// Binding an existing host after startup is not supported by this interface.
pub(crate) trait Owner: Send + Sync {
    fn capture<'a>(&'a self, identity: &'a Identity,
        configured_proxy: &'a Option<String>, path: &'static str, deadline: Instant)
        -> BoxFuture<'a, Result<Capture, CaptureError>>;
}

/// Only proven pre-start refusal or joined cleanup may release admission.
/// A timeout, dropped RPC waiter or metadata error alone is not that proof.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureError {
    pub failure: Failure,
    pub cleanup_confirmed: bool,
}

/// Truthful data retained by the host owner at the successful browser request.
/// Configured routes and constructed editor URLs cannot populate these fields.
/// No Debug/Clone: this is private context, not a reusable mint grant.
pub(crate) struct CapturedPage {
    /// Exact Rust-side transport incarnation from the successful bootstrap.
    pub sidecar_incarnation: uuid::Uuid,
    /// Exact retained HTTP-200 provenance UUID supplied by the host owner.
    pub provenance: String,
    pub application_generation: u64,
    pub host_generation: String,
    pub account_incarnation: u64,
    pub page_url: String,
    pub device_id: String,
    pub effective_proxy: Option<String>,
    pub wrapper_url: String,
    pub wrapper_export: String,
}

pub(crate) struct Capture {
    pub identity: Identity,
    /// Logical account configuration captured by the owner, including None.
    /// Distinct from the actual launch route, which may use a host fallback.
    /// The owner must bind its configuration revision to this same snapshot.
    pub configured_proxy: Option<String>,
    /// Exact settled SidecarBridge proxy-registration revision, possibly zero.
    /// Returned by the host crate's checked bootstrap, never inferred here.
    pub configured_proxy_revision: u64,
    pub sidecar: Arc<SidecarBridge>,
    pub native: NativeAccountClient,
    pub scope: AccountScope,
    pub page: CapturedPage,
    /// Explicit startup selection only; ordinary resident controls mint in-page.
    pub sdk: Option<Arc<offline_worker_owner::V2Worker>>,
}

/// Mark before any independent work. Unknown completion consumes capacity
/// and leaves both application/pool owners fenced, including panic or abort.
pub(super) struct Hold {
    pub admitted: Option<AdmittedControl>,
    settled: bool,
}
impl Hold {
    pub fn new(mut admitted: AdmittedControl) -> Self {
        admitted.pool.start();
        admitted.completion.start();
        Self { admitted: Some(admitted), settled: false }
    }
    pub fn admitted(&self) -> &AdmittedControl { self.admitted.as_ref().unwrap() }
    pub fn complete(mut self) -> bool {
        let admitted = self.admitted.take().unwrap();
        let AdmittedControl { capacity, pool, completion, .. } = admitted;
        if pool.complete().is_err() {
            capacity.forget();
            return false;
        }
        if completion.complete().is_err() {
            capacity.forget();
            return false;
        }
        self.settled = true;
        drop(capacity);
        true
    }
}
impl Drop for Hold {
    fn drop(&mut self) {
        if !self.settled {
            if let Some(admitted) = self.admitted.take() {
                admitted.capacity.forget();
                // Started pool and control drops retain uncertainty.
            }
        }
    }
}

pub(super) fn egress(proxy: &Option<String>) -> NativeEgress<'_> {
    proxy.as_deref().map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct)
}

pub(super) fn routes_match(scope: &AccountScope, configured: &Option<String>,
    admitted: &Option<String>, effective: &Option<String>) -> bool {
    configured == admitted && scope.matches_egress(&egress(effective))
}
