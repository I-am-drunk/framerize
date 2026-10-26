//! Account-bound offline SDK issuance. The concrete minter owns its worker.
use super::*;
use framer_core::ids::ProjectId;
use framer_core::port::CastleToken;

/// Actual application target. This constructor does not discover or invent it.
#[derive(Clone)]
pub struct OfflineMintTarget {
    project: ProjectId,
    path: &'static str,
    page_url: String,
}
impl OfflineMintTarget {
    pub fn new(target: CredentialTarget, page_url: String) -> Result<Self, String> {
        if !target.matches_offline_page(&page_url) {
            return Err("offline mint: target does not match project page".into());
        }
        Ok(Self { project: target.project, path: target.path, page_url })
    }
    pub fn project(&self) -> &ProjectId { &self.project }
    pub fn path(&self) -> &'static str { self.path }
    pub fn page_url(&self) -> &str { &self.page_url }
    fn matches(&self, target: &CredentialTarget) -> bool {
        self.project == target.project && self.path == target.path
    }
}

pub struct MintedCastle {
    scope: AccountScope,
    device: Option<String>,
    pub token: CastleToken,
    pub received_at: std::time::Instant,
}
impl MintedCastle {
    /// Preserve the worker owner's original generation and handoff time.
    /// This binding does not establish persistent SDK device continuity.
    pub fn new(scope: AccountScope, token: CastleToken,
        received_at: std::time::Instant) -> Self {
        Self { scope, device: None, token, received_at }
    }

    /// Carry the same saved device that seeded this invocation. The receiving
    /// source validates it against the private session before HTTP dispatch.
    pub fn with_saved_device(mut self, device: String) -> Self {
        self.device = Some(device);
        self
    }
}

/// One fresh, unmodified SDK output. No session or bearer enters the worker.
/// Completion must join actual work. Uncertainty must remain unconfirmed.
pub trait OfflineMinter: Send + Sync {
    /// Immutable owner of this minter, including its account generation.
    fn scope(&self) -> &AccountScope;
    /// Saved device already owned by this source, before credential acquisition.
    /// The joined mint must return the same device when one is declared here.
    fn saved_device(&self) -> Option<&str>;
    fn mint<'a>(&'a self, target: &'a OfflineMintTarget,
        control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<MintedCastle>>;
}

pub struct OfflineSessionSource {
    api: Arc<BoundControlClient>,
    target: OfflineMintTarget,
    minter: Arc<dyn OfflineMinter>,
    device: Option<String>,
}
impl OfflineSessionSource {
    /// Inert construction. Route, session and account authority stay upstream.
    pub fn new(api: Arc<BoundControlClient>, target: OfflineMintTarget,
        minter: Arc<dyn OfflineMinter>) -> Result<Self, String> {
        if !api.expected.same_generation(minter.scope()) {
            return Err("offline mint: minter account generation mismatch".into());
        }
        let device = minter.saved_device().map(str::to_owned);
        Ok(Self { api, target, minter, device })
    }

    async fn issue_bound(&self, scope: &AccountScope, session: &Session,
        target: &CredentialTarget, control: &PreparationControl)
        -> PreparationResult<PreparedCredentials> {
        if !self.api.expected.same_generation(scope)
            || !self.minter.scope().same_generation(scope)
            || scope.active_user() != &session.active_user
            || !self.target.matches(target)
            || !CurrentRequestCookies::valid_session(session) {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        let control_session = match self.control_session(session) {
            Ok(value) => value,
            Err(error) => return PreparationResult::finished(Err(error)),
        };
        // Keep the stored login unchanged; both requests use this saved device.
        let bearer = match self.api.bearer(scope, control_session.as_ref().unwrap_or(session)).await {
            Ok(value) => value,
            Err(_) => return PreparationResult::unconfirmed(),
        };
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        let minted = self.minter.mint(&self.target, control).await;
        self.finish_mint(scope, session, control, bearer, minted)
    }
}

impl OfflineSessionSource {
    fn control_session(&self, session: &Session) -> Result<Option<Session>, PreparationFailure> {
        self.device.as_deref().map(|device| {
            CurrentRequestCookies::from_saved_device(session, device)?.into_session(session)
        }).transpose().map_err(|_| PreparationFailure::Rejected)
    }

    fn finish_mint(&self, scope: &AccountScope, session: &Session, control: &PreparationControl,
        bearer: String, minted: PreparationResult<MintedCastle>)
        -> PreparationResult<PreparedCredentials> {
        if !minted.cleanup_confirmed { return PreparationResult::unconfirmed(); }
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        let value = minted.value.and_then(|minted| {
            if !minted.scope.same_generation(scope)
                || !self.minter.scope().same_generation(scope)
                || self.device.is_some() && minted.device.as_deref() != self.device.as_deref() {
                return Err(PreparationFailure::Rejected);
            }
            let current_cookies = minted.device.as_deref()
                .map(|device| CurrentRequestCookies::from_saved_device(session, device))
                .transpose().map_err(|_| PreparationFailure::Rejected)?;
            let token = minted.token;
            if token.0.is_empty() || token.0.len() > 16 * 1024 {
                return Err(PreparationFailure::Rejected);
            }
            let castle = FreshCastleToken::from_sdk_output_at(
                scope.account(), token, minted.received_at)
                .map_err(|_| PreparationFailure::Rejected)?
                .with_device(minted.device);
            Ok(PreparedCredentials {
                material: CredentialMaterial { bearer, castle },
                current_cookies,
            })
        });
        PreparationResult::finished(value)
    }
}

impl CredentialSource for OfflineSessionSource {
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

#[cfg(test)]
#[path = "offline_authorization_tests.rs"]
mod tests;
