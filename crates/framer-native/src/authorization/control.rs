//! Explicit one-shot authorization transport for the browser-backed control.
//! The native scope fixes the route. No process environment selects a source.
use crate::authorization::CredentialMaterial;
use crate::authorization::CredentialSource;
use crate::authorization::CredentialTarget;
use crate::authorization::CurrentRequestCookies;
use crate::authorization::PreparedCredentials;
use crate::account::client::AccountScope;
use crate::account::client::PreparationControl;
use crate::account::client::PreparationFailure;
use crate::account::client::PreparationResult;
use crate::transport::http::FreshCastleToken;
use crate::transport::http::NativeEgress;
use framer_core::port::BrowserBridge;
use framer_core::port::Session;
use futures::future::BoxFuture;
use futures::FutureExt;
use std::sync::Arc;





#[path = "resident_bearer_validity.rs"]
mod resident_bearer_validity;

struct ControlBearer {
    value: String,
    expires_at_ms: i64,
}

pub struct BoundControlClient {
    expected: AccountScope,
    transport: framer_api::session_auth::Client,
}
impl BoundControlClient {
    pub fn new(expected: AccountScope, egress: NativeEgress<'_>) -> Result<Self, String> {
        if !expected.matches_egress(&egress) {
            return Err("control authorization: account route mismatch".into());
        }
        let transport = framer_api::session_auth::Client::new(egress)?;
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
        let value = self.transport.ai_token(session).await?;
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

#[path = "offline_authorization.rs"]
mod offline_authorization;
pub use offline_authorization::MintedCastle;
pub use offline_authorization::OfflineMintTarget;
pub use offline_authorization::OfflineMinter;
pub use offline_authorization::OfflineSessionSource;

#[path = "browser_control.rs"]
mod browser_control;
#[path = "browser_control_owner.rs"]
pub(crate) mod browser_control_owner;
pub use browser_control::{BrowserControlMint, BrowserCredentialControl};
pub(crate) use browser_control::BrowserControlFactory;
#[cfg(test)]
pub(crate) use browser_control::tests as browser_control_fixture;

#[cfg(unix)]
#[path = "browser_control_wire.rs"]
mod browser_control_wire;
#[cfg(unix)]
#[path = "browser_control_process.rs"]
mod browser_control_process;
#[cfg(unix)]
pub use browser_control_process::{BrowserControlLaunch, BrowserControlProcess};
#[cfg(unix)]
pub use browser_control_wire::BrowserControlBinding;
