//! Shared transport for api.framer.com. Cookie auth for session calls, Bearer for
//! AI/web calls, and the status policy from lib.rs.

use framer_core::port::Session;
use serde_json::Value;

#[path = "control_response_evidence.rs"]
mod control_response_evidence;

#[cfg(test)]
#[path = "control_transport_tests.rs"]
mod control_transport_tests;

pub const API: &str = "https://api.framer.com";

pub struct Transport {
    http: reqwest::Client,
    base: String,
    // Legacy cached auth belongs to this transport. Native control_* calls
    // are uncached; shared session writers must carry this owner's handle.
    pub(crate) authorization: std::sync::Arc<crate::authorization::AuthorizationState>,
    pub(crate) messages_history: crate::chain::messages_history::HistoryStore,
}

pub enum Auth<'a> {
    Cookie(&'a Session),
    Bearer(&'a str),
    None,
}

fn normal_client_builder(
    connect_timeout: std::time::Duration,
    request_timeout: std::time::Duration,
) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent("Framerize/0.1")
        .connect_timeout(connect_timeout)
        .timeout(request_timeout)
}

// Do not format the error: it can contain a URL or proxy details.
fn control_transport_error(error: reqwest::Error) -> String {
    format!(
        "control transport failed: timeout={} connect={} request={} body={} decode={}",
        error.is_timeout(),
        error.is_connect(),
        error.is_request(),
        error.is_body(),
        error.is_decode(),
    )
}

impl Transport {
        pub(crate) fn for_account(egress: super::session_auth::Egress<'_>)
        -> Result<Self, String> {
        Self::explicit(API.to_owned(), egress, true)
    }

        fn explicit(base: String, egress: super::session_auth::Egress<'_>,
        https_only: bool) -> Result<Self, String> {
        use super::session_auth::Egress;
        let builder = reqwest::Client::builder().user_agent("Framerize/0.1")
            .no_proxy().redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never()).https_only(https_only).http1_only()
            .pool_max_idle_per_host(0)
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .no_gzip().no_brotli().no_deflate().no_zstd();
        let builder = match egress {
            Egress::Direct => builder,
            Egress::Proxy(url) => builder.proxy(reqwest::Proxy::all(url)
                .map_err(|_| "control authorization: invalid account proxy".to_owned())?),
        };
        let http = builder.build()
            .map_err(|_| "control authorization: client initialization failed".to_owned())?;
        Ok(Self { http, base, authorization: Default::default(), messages_history: Default::default() })
    }

    pub fn new() -> Self {
        Self::with_base(API.to_string())
    }

    /// Used by tests to point at the mock server.
    pub fn with_base(base: String) -> Self {
        let http = normal_client_builder(
            std::time::Duration::from_secs(10),
            std::time::Duration::from_secs(30),
        )
        .build()
        .expect("reqwest client");
        Self { http, base, authorization: Default::default(), messages_history: Default::default() }
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
        check(path, self.apply(req, auth).send().await.map_err(control_transport_error)?).await
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
        check(path, self.apply(req, auth).send().await.map_err(control_transport_error)?).await
    }

    pub async fn post_json(&self, path: &str, auth: Auth<'_>, body: Value) -> Result<Value, String> {
        self.post(path, auth, body)
            .await?
            .json()
            .await
            .map_err(|e| format!("decode {path}: {e}"))
    }

    /// Bounded, single-attempt response for the explicit control supplier.
    /// Failure messages contain no response body, cookie, URL or proxy.
        pub(crate) async fn control_ai_token(&self, session: &Session) -> Result<Value, String> {
        self.control_json("/auth/ai/access-token", Auth::Cookie(session)).await
    }

        pub(crate) async fn control_json(&self, path: &str, auth: Auth<'_>) -> Result<Value, String> {
        const LIMIT: usize = 64 * 1024;
        let (name, value, invalid) = match auth {
            Auth::Cookie(session) => (reqwest::header::COOKIE, session.cookie_header.clone(),
                "control authorization: invalid cookie"),
            Auth::Bearer(token) => (reqwest::header::AUTHORIZATION, format!("Bearer {token}"),
                "control authorization: invalid bearer"),
            Auth::None => return Err("control authorization: credentials required".into()),
        };
        let mut value = reqwest::header::HeaderValue::from_str(&value)
            .map_err(|_| invalid.to_owned())?;
        value.set_sensitive(true);
        let request = self.http.get(format!("{}{path}", self.base))
            .header("x-requested-by", "Framer").header(name, value);
        let mut response = request.send().await
            .map_err(|_| "control authorization: transport failed".to_owned())?;
        if path == "/auth/ai/access-token" {
            control_response_evidence::emit(response.status().as_u16(), response.headers());
        }
        if response.status().as_u16() != 200 {
            return Err(format!("control authorization: HTTP {}", response.status().as_u16()));
        }
        if response.content_length().is_some_and(|bytes| bytes > LIMIT as u64) {
            return Err("control authorization: response exceeds limit".to_owned());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await
            .map_err(|_| "control authorization: body failed".to_owned())? {
            if chunk.len() > LIMIT.saturating_sub(body.len()) {
                return Err("control authorization: response exceeds limit".to_owned());
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body)
            .map_err(|_| "control authorization: invalid response".to_owned())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn control_for_loopback(address: std::net::SocketAddr,
        egress: super::session_auth::Egress<'_>) -> Result<Self, String> {
        if !address.ip().is_loopback() { return Err("fixture requires loopback".into()); }
        Self::explicit(format!("http://{address}"), egress, false)
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
            let clipped = crate::error_body::prefix(&body, 500);
            Err(format!("retryable 403 on {path}: {clipped}"))
        }
        // Everything else: the response body is the only debugging signal we get
        // (a bare "retryable 400" hid why /auth/v4/signin rejected us once).
        s => {
            let body = resp.text().await.unwrap_or_default();
            let clipped = crate::error_body::prefix(&body, 500);
            Err(format!("retryable {s} on {path}: {clipped}"))
        }
    }
}
