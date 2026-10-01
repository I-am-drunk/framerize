//! REST/SSE server. Dumb by design: it renders state and forwards typed calls into
//! `framer-core`. No account logic lives here. Also serves the built web UI and the
//! compat surfaces for Claude Code (`/anthropic`) and Codex (`/openai`).

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use framer_core::account::ManagedAccount;
use framer_core::ids::AccountId;
use framer_core::pipeline::{self, Deps, InferenceOutcome, InferenceRequest};
use framer_core::port::{BrowserBridge, FramerClient, SessionStore, StateStore};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::services::ServeDir;

#[derive(Clone)]
pub struct AppState {
    pub client: Arc<dyn FramerClient>,
    pub browser: Arc<dyn BrowserBridge>,
    pub sessions: Arc<dyn SessionStore>,
    pub state: Arc<dyn StateStore>,
}

/// Build the router: REST under `/api`, compat under `/anthropic` + `/openai`,
/// everything else served from `web_dir`.
pub fn router(state: AppState, web_dir: &str) -> Router {
    let api = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/accounts", get(accounts))
        .route("/accounts/begin", post(begin_login))
        .route("/accounts/complete", post(complete_login))
        .route("/run", post(run))
        .with_state(state.clone());

    // Compat surfaces: thin translating proxies onto Framer's native dialect
    // routes. See reference/analysis/compat-mapping.md.
    let anthropic = Router::new()
        .route("/v1/messages", post(anthropic_messages))
        .with_state(state.clone());
    let openai = Router::new()
        .route("/v1/responses", post(openai_responses))
        .route("/v1/chat/completions", post(openai_chat))
        .with_state(state.clone());

    Router::new()
        .nest("/api", api)
        .nest("/anthropic", anthropic)
        .nest("/openai", openai)
        .fallback_service(ServeDir::new(web_dir))
}

pub async fn serve(state: AppState, web_dir: &str, addr: SocketAddr) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "framerize listening");
    axum::serve(listener, router(state, web_dir)).await
}

/// Shared compat forward: pick an eligible account, forward the native payload
/// through the matching gateway dialect, stream the upstream response back
/// untouched (same dialect the client sent — verified 2026-10-01).
///
/// Account continuity: clients may pin a conversation with `x-framerize-account`
/// + `x-framerize-session` headers; otherwise the least-failed eligible account
///   and a fresh chat session are used.
async fn compat_forward(
    s: AppState,
    dialect: &'static str,
    account_hdr: Option<String>,
    session_hdr: Option<String>,
    payload: serde_json::Value,
) -> Result<axum::response::Response, ApiError> {
    let pool = s.state.list().await.map_err(ApiError)?;
    let account = match account_hdr {
        Some(id) => pool
            .into_iter()
            .find(|a| a.id.0.to_string() == id)
            .ok_or_else(|| ApiError("unknown account id".into()))?,
        None => framer_core::rotation::pick(&pool)
            .cloned()
            .ok_or_else(|| ApiError("no eligible account".into()))?,
    };
    let session = s
        .sessions
        .load(&account.framer_user_id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError("account has no session".into()))?;
    let project = account
        .current_project
        .clone()
        .ok_or_else(|| ApiError("account has no current project".into()))?;
    let chat_session = session_hdr.unwrap_or_else(|| format!("s{:08x}", rand_u32()));

    let upstream = s
        .client
        .proxy(
            &session,
            s.browser.as_ref(),
            &account.id,
            &project,
            dialect,
            payload,
            &chat_session,
        )
        .await
        .map_err(ApiError)?;

    let status = StatusCode::from_u16(upstream.status)
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let body = axum::body::Body::from_stream(upstream.stream);
    let mut resp = axum::response::Response::new(body);
    *resp.status_mut() = status;
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        upstream.content_type.parse().unwrap_or(axum::http::HeaderValue::from_static("application/json")),
    );
    resp.headers_mut().insert(
        "x-framerize-session",
        chat_session.parse().unwrap_or(axum::http::HeaderValue::from_static("")),
    );
    Ok(resp)
}

fn rand_u32() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0))
        ^ std::process::id()
}

async fn anthropic_messages(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
    compat_forward(
        s,
        "anthropic-messages",
        hdr(&headers, "x-framerize-account"),
        hdr(&headers, "x-framerize-session"),
        payload,
    )
    .await
}

async fn openai_responses(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
    compat_forward(
        s,
        "openai-responses",
        hdr(&headers, "x-framerize-account"),
        hdr(&headers, "x-framerize-session"),
        payload,
    )
    .await
}

async fn openai_chat(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
    // OpenRouter chat route — observed UNMETERED (2026-10-01). Preferred.
    compat_forward(
        s,
        "openrouter-chat",
        hdr(&headers, "x-framerize-account"),
        hdr(&headers, "x-framerize-session"),
        payload,
    )
    .await
}

fn hdr(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
}

async fn accounts(State(s): State<AppState>) -> Result<Json<Vec<ManagedAccount>>, ApiError> {
    Ok(Json(s.state.list().await.map_err(ApiError)?))
}

#[derive(Deserialize)]
struct BeginLoginBody {
    account_id: AccountId,
    email: String,
}

/// Step 1 of adding an account: send the Framer magic-link email. The Castle token
/// comes from the account's (not yet authed) webview via the BrowserBridge.
async fn begin_login(
    State(s): State<AppState>,
    Json(body): Json<BeginLoginBody>,
) -> Result<&'static str, ApiError> {
    let castle = s
        .browser
        .castle_token(&body.account_id)
        .await
        .map_err(ApiError)?;
    s.client
        .request_magic_link(&body.email, &castle)
        .await
        .map_err(ApiError)?;
    Ok("sent")
}

#[derive(Deserialize)]
struct CompleteLoginBody {
    account_id: AccountId,
    /// The uuid from the emailed link's #token fragment.
    email_token: String,
}

/// Step 2: exchange the emailed token for the `session` cookie, store it in the
/// keychain. Links expire after 1 hour and are single-use.
async fn complete_login(
    State(s): State<AppState>,
    Json(body): Json<CompleteLoginBody>,
) -> Result<&'static str, ApiError> {
    let castle = s
        .browser
        .castle_token(&body.account_id)
        .await
        .map_err(ApiError)?;
    let session = s
        .client
        .complete_magic_link(&body.email_token, &castle)
        .await
        .map_err(ApiError)?;
    s.sessions.save(&session).await.map_err(ApiError)?;
    Ok("signed-in")
}

#[derive(Deserialize)]
struct RunBody {
    account_id: AccountId,
    prompt: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    chat_session: Option<String>,
}

async fn run(
    State(s): State<AppState>,
    Json(body): Json<RunBody>,
) -> Result<Json<InferenceOutcome>, ApiError> {
    let mut account = s
        .state
        .get(body.account_id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError("account not found".into()))?;

    let deps = Deps {
        client: s.client.as_ref(),
        browser: s.browser.as_ref(),
        sessions: s.sessions.as_ref(),
        state: s.state.as_ref(),
    };
    let req = InferenceRequest {
        prompt: body.prompt,
        model: body.model,
        chat_session: body.chat_session,
    };
    let outcome = pipeline::run(&deps, &mut account, &req)
        .await
        .map_err(|e| ApiError(e.to_string()))?;
    Ok(Json(outcome))
}

struct ApiError(String);

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.0).into_response()
    }
}
