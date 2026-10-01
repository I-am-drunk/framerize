//! REST/SSE server. Dumb by design: it renders state and forwards typed calls into
//! `framer-core`. No account logic lives here. Also serves the built web UI and the
//! compat surfaces for Claude Code (`/anthropic`) and Codex (`/openai`).

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use framer_core::account::ManagedAccount;
use framer_core::convert::{self, ClientDialect, CanonicalResponse};
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
        .route("/accounts/import", post(import_account))
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
    mut payload: serde_json::Value,
) -> Result<axum::response::Response, ApiError> {
    // Long non-streaming generations hit idle cutoffs (observed ~21.6 s in one
    // environment; a ~60 s server cutoff is plausible). ALWAYS stream upstream
    // and aggregate for clients that asked for stream:false.
    let client_wants_stream = payload["stream"].as_bool().unwrap_or(false);
    if !client_wants_stream {
        payload["stream"] = serde_json::Value::Bool(true);
    }

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

    if !client_wants_stream && upstream.content_type.contains("event-stream") {
        // Aggregate the SSE stream into the dialect's non-streaming JSON shape.
        use futures::StreamExt;
        let mut buf = Vec::new();
        let mut stream = upstream.stream;
        while let Some(chunk) = stream.next().await {
            buf.extend_from_slice(&chunk.map_err(ApiError)?);
        }
        let aggregated = aggregate_sse(dialect, &String::from_utf8_lossy(&buf));
        let mut resp = axum::response::Response::new(axum::body::Body::from(
            serde_json::to_string(&aggregated).map_err(|e| ApiError(e.to_string()))?,
        ));
        *resp.status_mut() = status;
        resp.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
        resp.headers_mut().insert(
            "x-framerize-session",
            chat_session.parse().unwrap_or(axum::http::HeaderValue::from_static("")),
        );
        return Ok(resp);
    }

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

/// Fold an upstream SSE stream into the dialect's non-streaming JSON response.
/// Verified event shapes in reference/analysis/verified-captures-2026-10-01.md.
fn aggregate_sse(dialect: &str, body: &str) -> serde_json::Value {
    let events: Vec<serde_json::Value> = body
        .split("\n\n")
        .filter_map(|block| {
            let data: String = block
                .lines()
                .filter_map(|l| l.strip_prefix("data:").map(|d| d.trim_start()))
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() || data == "[DONE]" {
                return None;
            }
            serde_json::from_str(&data).ok()
        })
        .collect();

    match dialect {
        // response.completed carries the entire final Responses object — use it.
        "openai-responses" => events
            .iter()
            .find(|e| e["type"] == "response.completed")
            .map(|e| e["response"].clone())
            .unwrap_or(serde_json::json!({"error": "stream ended without response.completed"})),
        // chat.completion.chunk deltas → one chat.completion.
        "openrouter-chat" => {
            let text: String = events
                .iter()
                .filter_map(|e| e["choices"][0]["delta"]["content"].as_str())
                .collect();
            let last = events.last().cloned().unwrap_or_default();
            let first = events.first().cloned().unwrap_or_default();
            let finish = events
                .iter()
                .find_map(|e| e["choices"][0]["finish_reason"].as_str())
                .unwrap_or("stop");
            serde_json::json!({
                "id": first["id"], "object": "chat.completion",
                "created": first["created"], "model": first["model"],
                "choices": [{"index": 0, "finish_reason": finish,
                             "message": {"role": "assistant", "content": text}}],
                "usage": last["usage"],
            })
        }
        // Anthropic: content_block_delta text + message_delta usage.
        _ => {
            let text: String = events
                .iter()
                .filter(|e| e["type"] == "content_block_delta")
                .filter_map(|e| e["delta"]["text"].as_str())
                .collect();
            let start = events
                .iter()
                .find(|e| e["type"] == "message_start")
                .cloned()
                .unwrap_or_default();
            let mut msg = start["message"].clone();
            msg["content"] = serde_json::json!([{"type": "text", "text": text}]);
            msg
        }
    }
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
    if hdr(&headers, "x-framerize-mode").as_deref() == Some("free") {
        return free_complete(s, &headers, ClientDialect::AnthropicMessages, payload).await;
    }
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
    if hdr(&headers, "x-framerize-mode").as_deref() == Some("free") {
        return free_complete(s, &headers, ClientDialect::OpenAiResponses, payload).await;
    }
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
    if hdr(&headers, "x-framerize-mode").as_deref() == Some("free") {
        return free_complete(s, &headers, ClientDialect::OpenAiChat, payload).await;
    }
    compat_forward(
        s,
        "openrouter-chat",
        hdr(&headers, "x-framerize-account"),
        hdr(&headers, "x-framerize-session"),
        payload,
    )
    .await
}

/// Free mode, ANY client dialect: parse the client's request into the canonical
/// IR, run the unmetered continuation chain on the OpenRouter chat route
/// (stream:false chunks — the upstream meter only hooks SSE; verified-captures
/// §6-8), then render the answer back into the client's dialect — as JSON, or
/// as a synthesized SSE stream when the client asked to stream. Adding a client
/// dialect = one module in framer_core::convert, never a change here.
async fn free_complete(
    s: AppState,
    headers: &axum::http::HeaderMap,
    dialect: ClientDialect,
    payload: serde_json::Value,
) -> Result<axum::response::Response, ApiError> {
    let req = convert::parse_request(dialect, &payload).map_err(ApiError)?;

    let pool = s.state.list().await.map_err(ApiError)?;
    let account = match hdr(headers, "x-framerize-account") {
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
    let chat_session =
        hdr(headers, "x-framerize-session").unwrap_or_else(|| format!("s{:08x}", rand_u32()));

    let cfg = framer_core::port::ChainConfig {
        max_total_tokens: Some(req.max_tokens),
        ..Default::default()
    };
    // Build the upstream chat payload (system-first messages, OpenRouter-prefixed
    // model id), then hand model+messages to the chainer — it owns chunking.
    let first = convert::to_chat_payload(&req, cfg.chunk_tokens);
    let model = first["model"].as_str().unwrap_or_default().to_string();
    let messages = first["messages"].as_array().cloned().unwrap_or_default();

    let result = s
        .client
        .freechat_long(
            &session,
            s.browser.as_ref(),
            &account.id,
            &project,
            &model,
            messages,
            &cfg,
            &chat_session,
        )
        .await
        .map_err(ApiError)?;

    let finish = if result.finish_reason.starts_with("length") {
        "length"
    } else {
        "stop"
    };
    let ir = CanonicalResponse {
        text: result.text,
        finish_reason: finish.into(),
        model: req.model.clone(), // echo the client's model id, not the upstream one
        prompt_tokens: result.prompt_tokens,
        completion_tokens: result.completion_tokens,
    };

    if req.stream {
        let frames = convert::render_stream(dialect, &ir);
        let body = frames
            .into_iter()
            .map(|f| format!("{f}\n\n"))
            .collect::<String>();
        let mut resp = axum::response::Response::new(axum::body::Body::from(body));
        resp.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
        resp.headers_mut().insert(
            "x-framerize-session",
            chat_session
                .parse()
                .unwrap_or(axum::http::HeaderValue::from_static("")),
        );
        return Ok(resp);
    }

    let body = convert::render_response(dialect, &ir);
    let mut resp = axum::response::Response::new(axum::body::Body::from(
        serde_json::to_string(&body).map_err(|e| ApiError(e.to_string()))?,
    ));
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    resp.headers_mut().insert(
        "x-framerize-session",
        chat_session
            .parse()
            .unwrap_or(axum::http::HeaderValue::from_static("")),
    );
    Ok(resp)
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
    // Mint + signin happen INSIDE the account's browser: api.framer.com
    // validates request/environment consistency (Origin/UA/TLS vs the Castle
    // token's fingerprint) and 400s the bare-HTTP-client variant — observed.
    s.browser
        .auth_begin(&body.account_id, &body.email)
        .await
        .map_err(ApiError)?;
    Ok("sent")
}

#[derive(Deserialize)]
struct CompleteLoginBody {
    account_id: AccountId,
    /// The uuid from the emailed link's #token fragment.
    email_token: String,
    /// Display alias for the account record (e.g. the email).
    alias: Option<String>,
    /// Project to scope inference to (20-char base62 id from the project URL).
    /// Can also be set later.
    project_id: Option<String>,
}

/// Step 2: exchange the emailed token for the `session` cookie, store it, then
/// register the account: /web/users/me gives the user id + workspaces, and the
/// ManagedAccount upsert makes inference routes work immediately after login.
/// Links expire after 1 hour and are single-use.
async fn complete_login(
    State(s): State<AppState>,
    Json(body): Json<CompleteLoginBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // In-page token exchange; returns the session cookie read out of the
    // browser jar via CDP plus the in-page /web/users/me payload.
    let (cookie_header, me) = s
        .browser
        .auth_complete(&body.account_id, &body.email_token)
        .await
        .map_err(ApiError)?;
    let user_id = me["id"]
        .as_str()
        .ok_or_else(|| ApiError("bad /web/users/me response".into()))?;
    let session = framer_core::port::Session {
        active_user: framer_core::ids::FramerUserId(user_id.to_string()),
        cookie_header,
    };
    s.sessions.save(&session).await.map_err(ApiError)?;
    let spaces = me["spaces"].as_array().cloned().unwrap_or_default();
    let first_org = spaces
        .iter()
        .find(|sp| sp["scope"].as_str() == Some("organization"))
        .cloned();
    let team = first_org
        .as_ref()
        .and_then(|sp| sp["ownerId"].as_str())
        .map(|t| framer_core::ids::TeamId(t.to_string()));

    let mut account = s
        .state
        .get(body.account_id)
        .await
        .map_err(ApiError)?
        .unwrap_or_else(|| ManagedAccount {
            id: body.account_id,
            alias: body.alias.clone().unwrap_or_else(|| "account".into()),
            framer_user_id: session.active_user.clone(),
            castle_device_id: uuid::Uuid::new_v4().to_string(),
            proxy: None,
            current_team: None,
            current_project: None,
            state: framer_core::account::AccountState::Ready,
            credit: None,
            consecutive_failures: 0,
            cooldown_step: 0,
        });
    account.framer_user_id = session.active_user.clone();
    account.state = framer_core::account::AccountState::Ready;
    if let Some(a) = &body.alias {
        account.alias = a.clone();
    }
    if team.is_some() {
        account.current_team = team;
    }
    if let Some(pid) = &body.project_id {
        account.current_project = Some(framer_core::ids::ProjectId(pid.clone()));
    }
    s.state.upsert(&account).await.map_err(ApiError)?;

    Ok(Json(serde_json::json!({
        "status": "signed-in",
        "account_id": account.id.0,
        "framer_user_id": account.framer_user_id.0,
        "team": account.current_team.map(|t| t.0),
        "project": account.current_project.map(|p| p.0),
    })))
}

#[derive(Deserialize)]
struct ImportBody {
    account_id: AccountId,
    /// Optional per-account egress proxy (residential etc.) — applied to the
    /// account's browser immediately.
    proxy: Option<String>,
    /// A live `session=…` cookie value (or full `session=…` header) captured
    /// from any browser already logged into the account. Login is the only
    /// IP-reputation-sensitive step; importing a session minted elsewhere
    /// lets a daemon on a flagged/blocked network operate the account.
    session_cookie: String,
    alias: Option<String>,
    project_id: Option<String>,
}

/// Import an externally-minted session cookie: verify it against
/// /web/users/me, then register the account exactly like complete_login.
async fn import_account(
    State(s): State<AppState>,
    Json(body): Json<ImportBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let cookie_header = if body.session_cookie.starts_with("session=") {
        body.session_cookie.clone()
    } else {
        format!("session={}", body.session_cookie)
    };
    // Probe with the cookie to learn the user id (transport-level; /web/*
    // reads are cookie-only and not Castle-gated).
    let probe = framer_core::port::Session {
        active_user: framer_core::ids::FramerUserId(String::new()),
        cookie_header: cookie_header.clone(),
    };
    let me = s.client.me(&probe).await.map_err(ApiError)?;
    let user_id = me["id"]
        .as_str()
        .ok_or_else(|| ApiError("bad /web/users/me response (cookie invalid?)".into()))?;
    let session = framer_core::port::Session {
        active_user: framer_core::ids::FramerUserId(user_id.to_string()),
        cookie_header,
    };
    s.sessions.save(&session).await.map_err(ApiError)?;

    let spaces = me["spaces"].as_array().cloned().unwrap_or_default();
    let first_org = spaces
        .iter()
        .find(|sp| sp["scope"].as_str() == Some("organization"))
        .cloned();
    let team = first_org
        .as_ref()
        .and_then(|sp| sp["ownerId"].as_str())
        .map(|t| framer_core::ids::TeamId(t.to_string()));

    let mut account = s
        .state
        .get(body.account_id)
        .await
        .map_err(ApiError)?
        .unwrap_or_else(|| ManagedAccount {
            id: body.account_id,
            alias: body.alias.clone().unwrap_or_else(|| "imported".into()),
            framer_user_id: session.active_user.clone(),
            castle_device_id: uuid::Uuid::new_v4().to_string(),
            proxy: None,
            current_team: None,
            current_project: None,
            state: framer_core::account::AccountState::Ready,
            credit: None,
            consecutive_failures: 0,
            cooldown_step: 0,
        });
    account.framer_user_id = session.active_user.clone();
    account.state = framer_core::account::AccountState::Ready;
    if let Some(a) = &body.alias {
        account.alias = a.clone();
    }
    if team.is_some() {
        account.current_team = team;
    }
    if let Some(pid) = &body.project_id {
        account.current_project = Some(framer_core::ids::ProjectId(pid.clone()));
    }
    if let Some(px) = &body.proxy {
        account.proxy = Some(px.clone());
        s.browser.set_proxy(&body.account_id, Some(px.clone())).await.map_err(ApiError)?;
    }
    s.state.upsert(&account).await.map_err(ApiError)?;

    Ok(Json(serde_json::json!({
        "status": "imported",
        "account_id": account.id.0,
        "framer_user_id": account.framer_user_id.0,
        "team": account.current_team.map(|t| t.0),
        "project": account.current_project.map(|p| p.0),
    })))
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
