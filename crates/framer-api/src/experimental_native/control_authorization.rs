//! Explicit one-shot authorization transport for the browser-backed control.
//! The native scope fixes the route. No process environment selects a source.
use super::{authorization::{CredentialMaterial, CredentialSource, CredentialTarget,
    CurrentRequestCookies, PreparedCredentials},
    native_account::{AccountScope, PreparationControl, PreparationFailure, PreparationResult},
    native_http::{FreshCastleToken, NativeEgress}};
use framer_core::port::{BrowserBridge, Session};
use futures::{future::BoxFuture, FutureExt};
use std::sync::Arc;

#[path = "contextual_control.rs"]
mod contextual_control;

#[path = "resident_control.rs"]
pub mod resident_control;

#[path = "credit_control.rs"]
mod credit_control;

#[cfg(test)]
#[path = "control_authorization_tests.rs"]
mod tests;

#[path = "resident_bearer_validity.rs"]
mod resident_bearer_validity;

struct ControlBearer {
    value: String,
    expires_at_ms: i64,
}

pub struct BoundControlClient {
    expected: AccountScope,
    transport: crate::transport::Transport,
}
impl BoundControlClient {
    pub fn new(expected: AccountScope, egress: NativeEgress<'_>) -> Result<Self, String> {
        if !expected.matches_egress(&egress) {
            return Err("control authorization: account route mismatch".into());
        }
        let transport = crate::transport::Transport::for_account(egress)?;
        Ok(Self { expected, transport })
    }

    async fn bearer(&self, scope: &AccountScope, session: &Session) -> Result<String, String> {
        self.bearer_with_expiry(scope, session).await.map(|bearer| bearer.value)
    }

    async fn bearer_with_expiry(&self, scope: &AccountScope, session: &Session)
        -> Result<ControlBearer, String> {
        if !self.expected.same_generation(scope) || scope.active_user() != &session.active_user {
            return Err("control authorization: account generation mismatch".into());
        }
        let value = self.transport.control_ai_token(session).await?;
        let bearer = value["accessToken"].as_str()
            .filter(|token| !token.is_empty() && token.len() <= 16 * 1024)
            .ok_or("control authorization: invalid token")?;
        let expires = value["expiresAt"].as_str()
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
            .ok_or("control authorization: invalid expiry")?;
        if expires.timestamp_millis() <= chrono::Utc::now().timestamp_millis() + 30_000 {
            return Err("control authorization: expired token".into());
        }
        Ok(ControlBearer { value: bearer.to_owned(), expires_at_ms: expires.timestamp_millis() })
    }
}

pub struct BoundBrowserSessionSource {
    api: Arc<BoundControlClient>,
    browser: Arc<dyn BrowserBridge>,
}
impl BoundBrowserSessionSource {
    pub fn new(api: Arc<BoundControlClient>, browser: Arc<dyn BrowserBridge>) -> Self {
        Self { api, browser }
    }
}

impl CredentialSource for BoundBrowserSessionSource {
    fn issue_for_target<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        target: &'a CredentialTarget, control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<PreparedCredentials>> {
        self.issue_contextual(scope, session, target, control).boxed()
    }

    fn issue<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        control: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async move {
            if !self.api.expected.same_generation(scope) || scope.active_user() != &session.active_user {
                return PreparationResult::finished(Err(PreparationFailure::Rejected));
            }
            if control.is_cancelled() {
                return PreparationResult::finished(Err(PreparationFailure::Cancelled));
            }
            let bearer = match self.api.bearer(scope, session).await {
                Ok(value) => value,
                Err(_) => return PreparationResult::unconfirmed(),
            };
            if control.is_cancelled() {
                return PreparationResult::finished(Err(PreparationFailure::Cancelled));
            }
            let account = scope.account();
            if self.browser.seed_session(&account, &session.cookie_header).await.is_err() {
                return PreparationResult::unconfirmed();
            }
            if control.is_cancelled() {
                return PreparationResult::finished(Err(PreparationFailure::Cancelled));
            }
            let token = match self.browser.castle_token(&account).await {
                Ok(value) if !value.0.is_empty() && value.0.len() <= 16 * 1024 => value,
                Ok(_) => return PreparationResult::finished(Err(PreparationFailure::Rejected)),
                Err(_) => return PreparationResult::unconfirmed(),
            };
            PreparationResult::finished(Ok(CredentialMaterial {
                bearer, castle: FreshCastleToken::from_browser(account, token),
            }))
        }.boxed()
    }
}

#[path = "offline_authorization.rs"]
mod offline_authorization;
pub use offline_authorization::{MintedCastle, OfflineMintTarget, OfflineMinter, OfflineSessionSource};
