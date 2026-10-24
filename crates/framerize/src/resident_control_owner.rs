//! Required private owner seams. No production implementation is supplied.
use super::*;
use framer_api::experimental_native::control_authorization::resident_control::ResidentBearer;
use framer_host::sidecar::SidecarBridge;
use serde_json::Value;
use uuid::Uuid;

/// The installed application owner must return already-owned, idle resources.
/// This method must be inert: no get-or-create, browser start, mint or private
/// store job. The native scope is transferred exclusively and will be closed.
/// Binding an existing host after startup is not supported by this interface.
pub(crate) trait Owner: Send + Sync {
    fn capture(&self, identity: &Identity) -> Result<Capture, Failure>;
}

/// Truthful data retained by the host owner at the successful browser request.
/// Configured routes and constructed editor URLs cannot populate these fields.
/// No Debug/Clone: this is private context, not a reusable mint grant.
pub(crate) struct CapturedPage {
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
    /// Its source is currently private to the host crate; no value is inferred.
    pub configured_proxy_revision: u64,
    pub sidecar: Arc<SidecarBridge>,
    pub native: NativeAccountClient,
    pub scope: AccountScope,
    pub page: CapturedPage,
    pub baseline: Arc<dyn BaselineOwner>,
}

pub(crate) struct BaselineInput<'a> {
    pub identity: &'a Identity,
    pub configured_proxy_revision: u64,
    pub page: &'a CapturedPage,
    pub lease: Uuid,
    pub path: &'static str,
    pub body: &'a Value,
    pub cookie_header: &'a str,
    pub bearer: &'a ResidentBearer,
}

/// Missing integration: an owned browser baseline under the SAME resident
/// lease and exact retained bearer. It must not mint/use the native token,
/// acquire a second bearer, start another observer or re-enter ordinary host
/// activity admission. Report after actual RPC/fetch settlement, even on stop.
/// The post-baseline observation must be freshly read under that lease.
/// No current resident RPC implements this action; do not simulate acceptance.
pub(crate) trait BaselineOwner: Send + Sync {
    fn run<'a>(&'a self, input: BaselineInput<'a>, control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<BaselineReceipt>>;
}

/// Private post-baseline snapshot from the same host operation, never caller
/// input. Identity fields are checked against the original lease before use.
pub(crate) struct BaselineReceipt {
    pub accepted: bool,
    pub status: Option<u16>,
    pub identity: Identity,
    pub configured_proxy_revision: u64,
    pub path: String,
    pub application_generation: u64,
    pub host_generation: String,
    pub account_incarnation: u64,
    pub lease: Uuid,
    pub page_url: String,
    pub device_id: String,
    pub cookie_header: String,
    pub effective_proxy: Option<String>,
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
