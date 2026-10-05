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

// Global per-(session,path) token cache (2026-10-05): the auth endpoints
// have a SUSTAINED-rate limiter (measured: ~300 mints in a burst -> 120/120
// 403s). Both token types are 600s JWTs; cache them and mint at most one per
// 570s per session+path, which makes the limiter unreachable in normal
// operation. The per-ACCOUNT cache in proxy.rs/inference.rs sits on top and
// is what actually fires per chain; this one is the backstop for every other
// caller (credits::usage mints a web token per call otherwise).
static TOKEN_CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<(String, String), AiToken>>> =
    std::sync::OnceLock::new();

fn cache() -> &'static std::sync::Mutex<std::collections::HashMap<(String, String), AiToken>> {
    TOKEN_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

async fn cached_mint(t: &Transport, session: &Session, path: &str) -> Result<AiToken, String> {
    let key = (session.cookie_header.clone(), path.to_string());
    if let Ok(guard) = cache().lock() {
        if let Some(tok) = guard.get(&key) {
            if tok.usable() {
                return Ok(tok.clone());
            }
        }
    }
    let tok = mint(t, session, path).await?;
    if let Ok(mut guard) = cache().lock() {
        guard.insert(key, tok.clone());
    }
    Ok(tok)
}

pub async fn ai_access_token(t: &Transport, session: &Session) -> Result<AiToken, String> {
    cached_mint(t, session, "/auth/ai/access-token").await
}

pub async fn web_access_token(t: &Transport, session: &Session) -> Result<AiToken, String> {
    cached_mint(t, session, "/auth/web/access-token").await
}
