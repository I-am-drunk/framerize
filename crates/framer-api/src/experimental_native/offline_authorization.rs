//! Account-bound offline SDK issuance. The concrete minter owns its worker.
use super::*;
use framer_core::{ids::ProjectId, port::CastleToken};

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
    pub token: CastleToken,
    pub received_at: std::time::Instant,
}
impl MintedCastle {
    /// Preserve the worker owner's original generation and handoff time.
    /// This binding does not establish persistent SDK device continuity.
    pub fn new(scope: AccountScope, token: CastleToken,
        received_at: std::time::Instant) -> Self {
        Self { scope, token, received_at }
    }
}

/// One fresh, unmodified SDK output. No session or bearer enters the worker.
/// Completion must join actual work. Uncertainty must remain unconfirmed.
pub trait OfflineMinter: Send + Sync {
    /// Immutable owner of this minter, including its account generation.
    fn scope(&self) -> &AccountScope;
    fn mint<'a>(&'a self, target: &'a OfflineMintTarget,
        control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<MintedCastle>>;
}

pub struct OfflineSessionSource {
    api: Arc<BoundControlClient>,
    target: OfflineMintTarget,
    minter: Arc<dyn OfflineMinter>,
}
impl OfflineSessionSource {
    /// Inert construction. Route, session and account authority stay upstream.
    pub fn new(api: Arc<BoundControlClient>, target: OfflineMintTarget,
        minter: Arc<dyn OfflineMinter>) -> Result<Self, String> {
        if !api.expected.same_generation(minter.scope()) {
            return Err("offline mint: minter account generation mismatch".into());
        }
        Ok(Self { api, target, minter })
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
        // Same private one-shot auth boundary as the reviewed browser control.
        let bearer = match self.api.bearer(scope, session).await {
            Ok(value) => value,
            Err(_) => return PreparationResult::unconfirmed(),
        };
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        let minted = self.minter.mint(&self.target, control).await;
        self.finish_mint(scope, control, bearer, minted)
    }
}

impl OfflineSessionSource {
    fn finish_mint(&self, scope: &AccountScope, control: &PreparationControl,
        bearer: String, minted: PreparationResult<MintedCastle>)
        -> PreparationResult<PreparedCredentials> {
        if !minted.cleanup_confirmed { return PreparationResult::unconfirmed(); }
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        let value = minted.value.and_then(|minted| {
            if !minted.scope.same_generation(scope)
                || !self.minter.scope().same_generation(scope) {
                return Err(PreparationFailure::Rejected);
            }
            let token = minted.token;
            if token.0.is_empty() || token.0.len() > 16 * 1024 {
                return Err(PreparationFailure::Rejected);
            }
            let castle = FreshCastleToken::from_sdk_output_at(
                scope.account(), token, minted.received_at)
                .map_err(|_| PreparationFailure::Rejected)?;
            Ok(PreparedCredentials {
                material: CredentialMaterial { bearer, castle },
                // No browser jar or invented device state is supplied here.
                current_cookies: None,
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
