//! Explicit experimental browser credentials. No default source selection.
use super::*;
use crate::account::device::DeviceGrant;
use framer_core::{ids::{AccountId, FramerUserId}, port::CastleToken};
use std::time::Instant;
use super::browser_control_owner::ControlDeviceOwner;
use crate::authorization::source::{Failure, SourceBuilder};
use crate::account::device_owner::DeviceOwner;
use crate::account::device::DeviceFailure;
use std::panic::AssertUnwindSafe;

/// A settled mint RPC. The peer still owns the browser control until finish.
/// Private credentials intentionally have no Debug implementation.
pub struct BrowserControlMint {
    pub account: AccountId,
    pub user: FramerUserId,
    pub generation: u64,
    pub host_generation: String,
    pub lease: uuid::Uuid,
    pub device: String,
    pub page_url: String,
    pub token: CastleToken,
    pub received_at: Instant,
}

/// One private peer. Successful mint settlement does not release its control.
/// Finish must join the consumer and physical cleanup before acknowledging.
/// Construction is inert. Prepare and mint have finite internal deadlines;
/// cancellation still joins actual work. The peer verifies the supplied
/// scope, session, grant, target and actual egress without logging credentials.
pub trait BrowserCredentialControl: Send + Sync {
    fn prepare(&self) -> BoxFuture<'_, bool>;
    fn mint<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        grant: &'a DeviceGrant, target: &'a OfflineMintTarget,
        budget_ms: u64, control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<BrowserControlMint>>;
    fn finish<'a>(&'a self, grant: &'a DeviceGrant, joined: bool, risk_refused: bool)
        -> BoxFuture<'a, bool>;
    fn shutdown(&self) -> BoxFuture<'_, bool>;
}

pub(crate) struct BrowserControlFactory {
    pub(crate) target: OfflineMintTarget,
    pub(crate) owner: Arc<ControlDeviceOwner>,
}

struct BrowserControlSource {
    scope: AccountScope,
    api: BoundControlClient,
    target: OfflineMintTarget,
    grant: Arc<DeviceGrant>,
    owner: Arc<ControlDeviceOwner>,
}

impl CredentialSource for BrowserControlSource {
    fn issue<'a>(&'a self, _: &'a AccountScope, _: &'a Session, _: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async { PreparationResult::finished(Err(PreparationFailure::Rejected)) }.boxed()
    }

    fn issue_for_target<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        target: &'a CredentialTarget, control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<PreparedCredentials>> {
        self.issue_bound(scope, session, target, control).boxed()
    }
}

#[cfg(test)]
#[path = "browser_control_tests.rs"]
pub(crate) mod tests;

impl SourceBuilder for BrowserControlFactory {
    fn build(&self, _: AccountScope, _: Option<String>)
        -> Result<Arc<dyn CredentialSource>, Failure> { Err(Failure::MissingAuthority) }

    fn device_owner(&self) -> Option<Arc<dyn DeviceOwner>> { Some(self.owner.clone()) }

    fn build_with_device(&self, scope: AccountScope, proxy: Option<String>,
        device: Option<Arc<DeviceGrant>>) -> Result<Arc<dyn CredentialSource>, Failure> {
        let grant = device.ok_or(Failure::MissingAuthority)?;
        if grant.account != scope.account() { return Err(Failure::MissingAuthority); }
        let route = proxy.as_deref().map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct);
        let api = BoundControlClient::new(scope.clone(), route).map_err(|_| Failure::Source)?;
        self.owner.claim(&grant).map_err(|_| Failure::MissingAuthority)?;
        Ok(Arc::new(BrowserControlSource {
            scope, api, grant, owner: self.owner.clone(), target: self.target.clone(),
        }))
    }
}

fn stopped(control: &PreparationControl) -> PreparationFailure {
    if tokio::time::Instant::now() >= control.deadline() { PreparationFailure::Deadline }
    else { PreparationFailure::Cancelled }
}

impl BrowserControlSource {
    async fn mint(&self, scope: &AccountScope, session: &Session, control: &PreparationControl)
        -> PreparationResult<BrowserControlMint> {
        let budget = control.deadline().saturating_duration_since(tokio::time::Instant::now())
            .as_millis().min(16_000) as u64;
        if budget == 0 { return PreparationResult::finished(Err(PreparationFailure::Deadline)); }
        let admitted = AssertUnwindSafe(async {
            self.owner.inner.admit_native_mint(&self.grant, budget).await
        }).catch_unwind().await;
        let sequence = match admitted {
            Ok(Ok(sequence)) => sequence,
            Ok(Err(DeviceFailure::Rejected)) =>
                return PreparationResult::finished(Err(PreparationFailure::Rejected)),
            _ => return PreparationResult::unconfirmed(),
        };
        let mut begun = false;
        let result = if control.is_cancelled() {
            PreparationResult::finished(Err(stopped(control)))
        } else if self.owner.begin(&self.grant).is_err() {
            PreparationResult::unconfirmed()
        } else {
            begun = true;
            AssertUnwindSafe(async {
                self.owner.peer.mint(scope, session, &self.grant, &self.target, budget, control).await
            }).catch_unwind().await.unwrap_or_else(|_| PreparationResult::unconfirmed())
        };
        self.finish_rate(sequence, result, begun).await
    }

    async fn finish_rate(&self, sequence: u64, result: PreparationResult<BrowserControlMint>,
        begun: bool) -> PreparationResult<BrowserControlMint> {
        let finished = AssertUnwindSafe(async {
            self.owner.inner.finish_native_mint(&self.grant, sequence,
                result.cleanup_confirmed).await
        }).catch_unwind().await;
        let settled = result.cleanup_confirmed && matches!(finished, Ok(Ok(_)));
        if begun && self.owner.minted(&self.grant, settled).is_err() {
            return PreparationResult::unconfirmed();
        }
        if !settled { return PreparationResult::unconfirmed(); }
        if matches!(finished, Ok(Ok(false))) && result.value.is_ok() {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        result
    }
}

impl BrowserControlSource {
    async fn issue_bound(&self, scope: &AccountScope, session: &Session,
        target: &CredentialTarget, control: &PreparationControl)
        -> PreparationResult<PreparedCredentials> {
        if !self.scope.same_generation(scope) || scope.active_user() != &session.active_user
            || self.target.project() != &target.project || self.target.path() != target.path {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        if control.is_cancelled() { return PreparationResult::finished(Err(stopped(control))); }
        let remaining = control.deadline().saturating_duration_since(tokio::time::Instant::now());
        if remaining > std::time::Duration::from_secs(180) {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        let current = match CurrentRequestCookies::from_saved_device(session, &self.grant.device) {
            Ok(value) => value,
            Err(_) => return PreparationResult::finished(Err(PreparationFailure::Rejected)),
        };
        let wire_session = match current.into_session(session) {
            Ok(value) => value,
            Err(_) => return PreparationResult::finished(Err(PreparationFailure::Rejected)),
        };
        if self.owner.issue(&self.grant).is_err() {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        let bearer = match self.api.bearer_with_expiry(scope, &wire_session).await {
            Ok(value) => value, Err(_) => return PreparationResult::unconfirmed(),
        };
        self.issue_after_bearer(scope, session, &wire_session, control, bearer).await
    }

    async fn issue_after_bearer(&self, scope: &AccountScope, session: &Session,
        wire_session: &Session, control: &PreparationControl, bearer: ControlBearer)
        -> PreparationResult<PreparedCredentials> {
        if control.is_cancelled() { return PreparationResult::finished(Err(stopped(control))); }
        let remaining = control.deadline().saturating_duration_since(tokio::time::Instant::now());
        if !resident_bearer_validity::covers_resident_deadline(
            bearer.expires_at_ms, chrono::Utc::now().timestamp_millis(), remaining) {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        let minted = self.mint(scope, wire_session, control).await;
        if !minted.cleanup_confirmed { return PreparationResult::unconfirmed(); }
        if control.is_cancelled() { return PreparationResult::finished(Err(stopped(control))); }
        PreparationResult::finished(minted.value.and_then(|mint| {
            if mint.account != scope.account() || mint.user != *scope.active_user()
                || mint.generation != scope.generation() || mint.lease != self.grant.lease
                || mint.host_generation != self.grant.host_generation
                || mint.device != self.grant.device || mint.page_url != self.target.page_url()
                || mint.token.0.is_empty() || mint.token.0.len() > 16 * 1024 {
                return Err(PreparationFailure::Rejected);
            }
            let castle = FreshCastleToken::from_sdk_output_at(scope.account(), mint.token,
                mint.received_at).map_err(|_| PreparationFailure::Rejected)?
                .with_device(Some(mint.device));
            let current_cookies = Some(CurrentRequestCookies::from_saved_device(session,
                &self.grant.device).map_err(|_| PreparationFailure::Rejected)?);
            Ok(PreparedCredentials { material: CredentialMaterial { bearer: bearer.value, castle },
                current_cookies })
        }))
    }
}
