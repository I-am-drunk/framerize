//! Token minting. GET /auth/ai/access-token (cookie-authed) →
//! {accessToken, expiresAt, expiresInSeconds}: a 600 s RS256 JWT whose payload
//! carries userId/email/name (handy for labeling accounts).
//! GET /auth/web/access-token is the same shape for /web/* reads.
//! The app caches and refreshes ~30 s before expiry; so do we (per account).

use crate::transport::{Auth, Transport};
use framer_core::port::Session;

#[derive(Clone)]
pub struct AiToken {
    pub bearer: String,
    pub expires_at_ms: i64,
}

impl AiToken {
    pub fn usable(&self) -> bool {
        // 30 s skew, mirroring the app's refresher.
        self.expires_at_ms > now_ms() + 30_000
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

async fn mint(t: &Transport, session: &Session, path: &str) -> Result<AiToken, String> {
    let v = t.get(path, Auth::Cookie(session)).await?.json::<serde_json::Value>().await.map_err(|e| e.to_string())?;
    let bearer = v["accessToken"].as_str().ok_or("bad access-token response")?.to_string();
    let expires_at_ms = v["expiresAt"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .ok_or("bad expiresAt")?;
    Ok(AiToken { bearer, expires_at_ms })
}

pub async fn ai_access_token(t: &Transport, session: &Session) -> Result<AiToken, String> {
    mint(t, session, "/auth/ai/access-token").await
}

pub async fn web_access_token(t: &Transport, session: &Session) -> Result<AiToken, String> {
    mint(t, session, "/auth/web/access-token").await
}
