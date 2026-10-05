//! Shared transport for api.framer.com. Cookie auth for session calls, Bearer for
//! AI/web calls, and the status policy from lib.rs.

use framer_core::port::Session;
use serde_json::Value;

pub const API: &str = "https://api.framer.com";

pub struct Transport {
    http: reqwest::Client,
    base: String,
}

pub enum Auth<'a> {
    Cookie(&'a Session),
    Bearer(&'a str),
    None,
}

impl Transport {
    pub fn new() -> Self {
        Self::with_base(API.to_string())
    }

    /// Used by tests to point at the mock server.
    pub fn with_base(base: String) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("Framerize/0.1")
            .build()
            .expect("reqwest client");
        Self { http, base }
    }

    fn apply(&self, req: reqwest::RequestBuilder, auth: Auth) -> reqwest::RequestBuilder {
        // Every API call in Framer's own client carries this constant header
        // (chunk-3MN6NPEY: Object.freeze({"X-Requested-By":"Framer"}), spread
        // into the shared fetch wrapper). It's the API's CSRF marker — omitting
        // it gets a bare 400 on /auth/v4/* (observed live on signin).
        let req = req.header("x-requested-by", "Framer");
        match auth {
            Auth::Cookie(s) => req.header("cookie", &s.cookie_header),
            Auth::Bearer(t) => req.bearer_auth(t),
            Auth::None => req,
        }
    }

    pub async fn get(&self, path: &str, auth: Auth<'_>) -> Result<reqwest::Response, String> {
        let req = self.http.get(format!("{}{}", self.base, path));
        check(path, self.apply(req, auth).send().await.map_err(|e| e.to_string())?).await
    }

    pub async fn post(
        &self,
        path: &str,
        auth: Auth<'_>,
        body: Value,
    ) -> Result<reqwest::Response, String> {
        let req = self
            .http
            .post(format!("{}{}", self.base, path))
            .header("content-type", "application/json")
            .json(&body);
        check(path, self.apply(req, auth).send().await.map_err(|e| e.to_string())?).await
    }

    pub async fn post_json(&self, path: &str, auth: Auth<'_>, body: Value) -> Result<Value, String> {
        self.post(path, auth, body)
            .await?
            .json()
            .await
            .map_err(|e| format!("decode {path}: {e}"))
    }
}

/// Status policy — see AGENTS.md. Returns the response on success.
/// 401/402/403 map to machine-readable strings the pipeline matches on.
async fn check(path: &str, resp: reqwest::Response) -> Result<reqwest::Response, String> {
    match resp.status().as_u16() {
        200..=299 => Ok(resp),
        // Exact-match sentinels the pipeline matches on — do not append to these.
        401 => Err("unauthorized".into()),
        402 => Err("credits-exhausted".into()),
        // Bare HTTP path (auth/credits), not the AI passthrough. A 403 on an
        // /auth/* endpoint is the auth rate limiter (measured 2026-10-05: the
        // single-slot JWT cache thrash drove /auth/ai/access-token to
        // 120/120 403s) — it is NOT Castle scoring the account, and labeling
        // it "risk-refused" poisons account health with phantom refusals.
        // Only /ai/v3/* 403s go through Castle classification.
        403 => {
            let body = resp.text().await.unwrap_or_default();
            if path.starts_with("/ai/") {
                return Err(crate::classify::classify_403(&body, 0).as_error(path, &body));
            }
            let clipped = &body[..body.len().min(500)];
            Err(format!("retryable 403 on {path}: {clipped}"))
        }
        // Everything else: the response body is the only debugging signal we get
        // (a bare "retryable 400" hid why /auth/v4/signin rejected us once).
        s => {
            let body = resp.text().await.unwrap_or_default();
            let clipped = &body[..body.len().min(500)];
            Err(format!("retryable {s} on {path}: {clipped}"))
        }
    }
}
