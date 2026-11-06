//! Token minting. GET /auth/ai/access-token (cookie-authed) →
//! {accessToken, expiresAt, expiresInSeconds}: a 600 s RS256 JWT whose payload
//! carries userId/email/name (handy for labeling accounts).
//! GET /auth/web/access-token is the same shape for /web/* reads.
//! The app caches and refreshes ~30 s before expiry; so do we (per account).

use crate::transport::{Auth, Transport};
use framer_core::port::Session;

#[cfg(test)]
#[path = "auth_retry_tests.rs"]
mod retry_tests;

#[derive(Clone)]
pub struct AiToken {
    pub bearer: String,
    pub expires_at_ms: i64,
    // An account ID can outlive the login that authorized this bearer.
    session: Session,
}

impl AiToken {
    pub(crate) fn for_session(session: &Session, bearer: String, expires_at_ms: i64) -> Self {
        Self { bearer, expires_at_ms, session: session.clone() }
    }

    fn belongs_to(&self, session: &Session) -> bool {
        self.session.active_user == session.active_user
            && self.session.cookie_header == session.cookie_header
    }

    pub(crate) fn usable_for(&self, session: &Session) -> bool {
        self.belongs_to(session) && self.usable()
    }

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
    #[cfg(test)]
    if let Some(result) = retry_tests::mint_fixture(session, path).await {
        return result;
    }
    // JWT relay (2026-10-13): when the host's egress IP is edge-WAF-blocked
    // on /auth/* (measured: household 403 CloudFront block while datacenter
    // IPs mint fine), an external relay mints JWTs and publishes them at a
    // URL. Fetch first; fall back to the local mint on ANY relay failure, so
    // the relay can never take the daemon down with it.
    if path == "/auth/ai/access-token"
        && std::env::var("FRAMERIZE_JWT_RELAY_URL").is_ok_and(|url| !url.is_empty()) {
        match relay_mint(session).await {
            Ok(Some(tok)) => return Ok(tok),
            Ok(None) => {}, // This relay does not serve this account.
            Err(e) => eprintln!(
                "[auth] jwt relay fetch failed for {} ({}) — falling back to local mint",
                &session.active_user.0.get(..8).unwrap_or("?"),
                e
            ),
        }
    }
    let v = t.get(path, Auth::Cookie(session)).await?.json::<serde_json::Value>().await.map_err(|e| e.to_string())?;
    let bearer = v["accessToken"].as_str().ok_or("bad access-token response")?.to_string();
    let expires_at_ms = v["expiresAt"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .ok_or("bad expiresAt")?;
    Ok(AiToken::for_session(session, bearer, expires_at_ms))
}


/// Fetch a relayed JWT for this session's user from FRAMERIZE_JWT_RELAY_URL.
/// The relay publishes `{accounts: {<user_id>: {accessToken, expiresAt}}}`.
/// Auth to the relay itself comes from FRAMERIZE_JWT_RELAY_TOKEN (sent as a
/// bearer header — e.g. a PAT for a private GitHub raw URL).
async fn relay_mint(session: &Session) -> Result<Option<AiToken>, String> {
    let relay = std::env::var("FRAMERIZE_JWT_RELAY_URL").unwrap_or_default();
    let token = std::env::var("FRAMERIZE_JWT_RELAY_TOKEN").ok();
    relay_mint_from(session, &relay, token.as_deref()).await
}

async fn relay_mint_from(session: &Session, relay: &str, token: Option<&str>)
    -> Result<Option<AiToken>, String> {
    if relay.is_empty() {
        return Err("no relay configured".into());
    }
    // Contents-API form avoids raw.githubusercontent.com's ~5min edge cache
    // (a 600s JWT cannot tolerate stale relay files). The `r` param is a
    // cache-buster regardless.
    let mut url = reqwest::Url::parse(&relay).map_err(|_| "invalid JWT relay URL")?;
    url.query_pairs_mut().append_pair("r", &now_ms().to_string());
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client
        .get(url)
        .header("accept", "application/vnd.github.raw")
        .header("user-agent", "framerize-relay/0.1");
    if let Some(tok) = token {
        if !tok.is_empty() {
            req = req.header("authorization", format!("Bearer {}", tok));
        }
    }
    let response = req.send().await.map_err(|e| e.without_url().to_string())?;
    if !response.status().is_success() {
        return Err(format!("JWT relay returned HTTP {}", response.status().as_u16()));
    }
    let v: serde_json::Value = response.json().await.map_err(|e| e.without_url().to_string())?;
    let accounts = v["accounts"].as_object().ok_or("JWT relay response has no accounts object")?;
    let Some(entry) = accounts.get(&session.active_user.0) else { return Ok(None); };
    let bearer = entry["accessToken"].as_str().ok_or("relay: no token for user")?.to_string();
    let expires_at_ms = entry["expiresAt"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .ok_or("relay: bad expiresAt")?;
    let token = AiToken::for_session(session, bearer, expires_at_ms);
    if !token.usable() {
        return Err("relay: token is expired or within the refresh window".into());
    }
    Ok(Some(token))
}

#[cfg(test)]
#[path = "auth_relay_tests.rs"]
mod relay_tests;

// Runtime-owned per-(session,path) cache: the auth endpoints
// have a SUSTAINED-rate limiter (measured: ~300 mints in a burst -> 120/120
// 403s). Both token types are 600s JWTs; cache them and mint at most one per
// 570s per session+path, which makes the limiter unreachable in normal
// operation. The per-ACCOUNT cache in proxy.rs/inference.rs sits on top and
// is what actually fires per chain; this one is the backstop for every other
// caller (credits::usage mints a web token per call otherwise).
async fn cached_mint(t: &Transport, session: &Session, path: &str) -> Result<AiToken, String> {
    let key = (session.cookie_header.clone(), path.to_string());
    if let Ok(guard) = t.authorization.sessions.lock() {
        if let Some(tok) = guard.get(&key) {
            if tok.usable_for(session) {
                return Ok(tok.clone());
            }
        }
    }
    let lock_key = (session.active_user.0.clone(), path.to_string());
    let mint_lock = {
        let mut locks = t.authorization.mint_locks.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&lock_key).and_then(|lock| lock.upgrade()) {
            lock
        } else {
            let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(lock_key, std::sync::Arc::downgrade(&lock));
            lock
        }
    };
    let _mint_guard = mint_lock.lock().await;
    if let Ok(guard) = t.authorization.sessions.lock() {
        if let Some(tok) = guard.get(&key) {
            if tok.usable_for(session) {
                return Ok(tok.clone());
            }
        }
    }
    let tok = mint(t, session, path).await?;
    if let Ok(mut guard) = t.authorization.sessions.lock() {
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

/// A rejected AI bearer must not survive in the shared session cache. A late
/// rejection must also leave a newer bearer and the web-token cache intact.
pub(crate) fn invalidate_ai_token(t: &Transport, session: &Session, rejected: &AiToken) {
    let key = (session.cookie_header.clone(), "/auth/ai/access-token".to_owned());
    let mut guard = t.authorization.sessions.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.get(&key).is_some_and(|current|
        current.belongs_to(session) && current.bearer == rejected.bearer) {
        guard.remove(&key);
    }
}
