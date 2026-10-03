//! REST/SSE server. Dumb by design: it renders state and forwards typed calls into
//! `framer-core`. No account logic lives here. Also serves the built web UI and the
//! compat surfaces for Claude Code (`/anthropic`) and Codex (`/openai`).

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use framer_core::account::ManagedAccount;
use framer_core::convert::{self, ClientDialect};
use framer_core::ids::AccountId;
use framer_core::pipeline::{self, Deps, InferenceOutcome, InferenceRequest};
use framer_core::port::{BrowserBridge, FramerClient, SessionStore, StateStore};
use serde::Deserialize;
use serde_json::Value;
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
        // Claude Code connection-warming probe (gateway protocol: safe to
        // answer or reject; we answer 200).
        .route("/hello", get(|| async { "ok" }).head(|| async { "ok" }))
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
        // Claude Code gateway protocol: token counting is optional (the client
        // falls back to a character estimate when absent), but cheap to serve.
        .route("/v1/messages/count_tokens", post(count_tokens))
        .with_state(state.clone());
    let openai = Router::new()
        .route("/v1/responses", post(openai_responses))
        .route("/v1/chat/completions", post(openai_chat))
        .with_state(state.clone());

    // Convention: the BARE compat routes are the free path (unmetered chain),
    // and /paid/* holds the metered streaming passthroughs. Claude Code/Codex
    // can't send custom headers, so defaulting free at the normal path means
    // zero client config beyond the base URL; x-framerize-mode: paid remains
    // as a header override for clients that can set it.
    let anthropic_paid = Router::new()
        .route("/v1/messages", post(anthropic_messages_paid))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .with_state(state.clone());
    let openai_paid = Router::new()
        .route("/v1/responses", post(openai_responses_paid))
        .route("/v1/chat/completions", post(openai_chat_paid))
        .with_state(state.clone());
    let paid = Router::new()
        .nest("/anthropic", anthropic_paid)
        .nest("/openai", openai_paid);

    Router::new()
        .nest("/api", api)
        .nest("/anthropic", anthropic)
        .nest("/openai", openai)
        .nest("/paid", paid)
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
    if hdr(&headers, "x-framerize-mode").as_deref() != Some("paid") {
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
    if hdr(&headers, "x-framerize-mode").as_deref() != Some("paid") {
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
    if hdr(&headers, "x-framerize-mode").as_deref() != Some("paid") {
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

/// Free mode, ANY client dialect, LOSSLESS: convert the client's request to
/// Anthropic Messages shape with full fidelity (tools, system, tool_choice,
/// content blocks — convert::to_anthropic_request), run the unmetered
/// continuation chain on /ai/v3/messages/ (block-aware), then render the
/// assembled response back into the client's dialect — as JSON, or as a
/// synthesized SSE stream when the client asked to stream.
async fn free_complete(
    s: AppState,
    headers: &axum::http::HeaderMap,
    dialect: ClientDialect,
    payload: serde_json::Value,
) -> Result<axum::response::Response, ApiError> {
    let client_model = payload["model"].as_str().unwrap_or_default().to_string();
    let wants_stream = payload["stream"].as_bool().unwrap_or(false);
    let include_usage = payload["stream_options"]["include_usage"].as_bool().unwrap_or(false);
    let max_tokens = payload["max_tokens"].as_u64()
        .or_else(|| payload["max_completion_tokens"].as_u64())
        .or_else(|| payload["max_output_tokens"].as_u64())
        .unwrap_or(4096) as u32;

    // Client request → faithful Anthropic Messages payload (model normalized).
    let anthro = convert::to_anthropic_request(dialect, &payload).map_err(ApiError)?;

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
        max_total_tokens: Some(max_tokens),
        // Metered rungs are credit spend: off unless the operator opts in.
        metered_fallback: std::env::var("FRAMERIZE_ALLOW_METERED").map(|v| v == "1").unwrap_or(false),
        ..Default::default()
    };
    if wants_stream {
        // INCREMENTAL streaming: spawn the chain with an events callback and
        // emit SSE frames as chunks land — message_start/role/created
        // immediately, per-chunk deltas as they arrive, heartbeat pings every
        // 8s of silence. The old behavior buffered the entire chain before
        // the first byte; streaming clients (Claude Code) timed out the
        // silent socket: "Claude gave up after repeated API errors".
        let msg_id = format!("msg_{:08x}{:08x}", rand_u32(), rand_u32());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<convert::ChainEvent>(64);
        let s2 = s.clone();
        let account2 = account.clone();
        let project2 = project.clone();
        let chat_session2 = chat_session.clone();
        let log_model = anthro_model_for_log(dialect, &payload);
        let mut handle = tokio::spawn(async move {
            let t0 = std::time::Instant::now();
            let cb = move |ev: convert::ChainEvent| {
                let _ = tx.try_send(ev);
            };
            let result = s2
                .client
                .freechat_long(
                    &session,
                    s2.browser.as_ref(),
                    &account2.id,
                    &project2,
                    anthro,
                    &cfg,
                    &chat_session2,
                    Some(&cb),
                )
                .await;
            {
                use framer_core::observability::Event;
                let mut ev = Event::new("ai.chain")
                    .account(&account2.id)
                    .field("model", log_model)
                    .field("dialect", format!("{dialect:?}"))
                    .duration_ms(t0.elapsed().as_millis());
                ev = match &result {
                    Ok(r) => ev
                        .field("chunks", r.chunks)
                        .field("metered_chunks", r.metered_chunks)
                        .field("prompt_tokens", r.input_tokens)
                        .field("completion_tokens", r.output_tokens)
                        .field("finish_reason", r.stop_reason.clone())
                        .outcome("ok"),
                    Err(e) => ev.error(e),
                };
                ev.emit();
            }
            result
        });

        let open_frames = convert::stream_open(dialect, &client_model, &msg_id);
        let composer = convert::SseComposer::new(dialect, &msg_id, &client_model);
        struct St {
            done: bool,
        }
        let stream = futures::stream::unfold(
            (rx, open_frames, composer, St { done: false }, handle, client_model, include_usage, dialect),
            |(mut rx, mut pending, mut composer, mut st, mut handle, client_model, include_usage, dialect)| async move {
                if st.done {
                    return None;
                }
                if !pending.is_empty() {
                    let out = pending.join("\n\n") + "\n\n";
                    pending.clear();
                    return Some((Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(out)), (rx, pending, composer, st, handle, client_model, include_usage, dialect)));
                }
                let frames: Vec<String> = tokio::select! {
                    biased;
                    r = &mut handle => {
                        st.done = true;
                        // DRAIN pending chunk events BEFORE finishing: a biased
                        // select resolves the handle first when the last chunk
                        // and the completion race — without this the final
                        // chunk's frames (the tool_use block!) are dropped and
                        // the client sees stop_reason=tool_use with no tool call.
                        let mut drained: Vec<String> = Vec::new();
                        while let Ok(ev) = rx.try_recv() {
                            if let convert::ChainEvent::Chunk(blocks) = ev {
                                drained.extend(composer.push_chunk(&blocks));
                            }
                        }
                        let tail = match r {
                            Ok(Ok(assembled)) => composer.finish(&assembled, &client_model, include_usage),
                            Ok(Err(e)) => {
                                let err_resp = convert::AssembledResponse {
                                    id: "msg_err".into(),
                                    content: vec![serde_json::json!({"type":"text","text":format!("[framerize] {e}")})],
                                    stop_reason: "end_turn".into(),
                                    stop_sequence: None,
                                    usage: serde_json::json!({}),
                                    input_tokens: 0, output_tokens: 0, chunks: 0, metered_chunks: 0,
                                };
                                composer.push_chunk(&err_resp.content.clone())
                                    .into_iter()
                                    .chain(composer.finish(&err_resp, &client_model, false))
                                    .collect()
                            }
                            Err(je) => vec![convert::ping_frame(dialect), format!(": join error {je}")],
                        };
                        drained.into_iter().chain(tail).collect()
                    }
                    ev = rx.recv() => match ev {
                        Some(convert::ChainEvent::Chunk(blocks)) => composer.push_chunk(&blocks),
                        Some(convert::ChainEvent::Done(_)) => vec![],
                        None => vec![],
                    },
                    _ = tokio::time::sleep(std::time::Duration::from_secs(8)) => {
                        vec![convert::ping_frame(dialect)]
                    }
                };
                if frames.is_empty() && !st.done {
                    return Some((Ok(axum::body::Bytes::new()), (rx, pending, composer, st, handle, client_model, include_usage, dialect)));
                }
                let out = if frames.is_empty() { String::new() } else { frames.join("\n\n") + "\n\n" };
                Some((Ok(axum::body::Bytes::from(out)), (rx, pending, composer, st, handle, client_model, include_usage, dialect)))
            },
        );
        let mut resp = axum::response::Response::new(axum::body::Body::from_stream(stream));
        resp.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
        resp.headers_mut().insert(
            "x-framerize-session",
            chat_session.parse().unwrap_or(axum::http::HeaderValue::from_static("")),
        );
        resp.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        );
        return Ok(resp);
    }

    let t0 = std::time::Instant::now();
    let result = s
        .client
        .freechat_long(
            &session,
            s.browser.as_ref(),
            &account.id,
            &project,
            anthro,
            &cfg,
            &chat_session,
            None,
        )
        .await;

    // canonical ai.chain event (credit-use proof lives here: metered_chunks)
    {
        use framer_core::observability::Event;
        let mut ev = Event::new("ai.chain")
            .account(&account.id)
            .field("model", anthro_model_for_log(dialect, &payload))
            .field("dialect", format!("{dialect:?}"))
            .duration_ms(t0.elapsed().as_millis());
        ev = match &result {
            Ok(r) => ev
                .field("chunks", r.chunks)
                .field("metered_chunks", r.metered_chunks)
                .field("prompt_tokens", r.input_tokens)
                .field("completion_tokens", r.output_tokens)
                .field("finish_reason", r.stop_reason.clone())
                .outcome("ok"),
            Err(e) => ev.error(e),
        };
        ev.emit();
    }
    let result = result.map_err(ApiError)?;

    let body = convert::render_json(dialect, &result, &client_model);
    let mut resp = axum::response::Response::new(axum::body::Body::from(
        serde_json::to_string(&body).map_err(|e| ApiError(e.to_string()))?,
    ));
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    resp.headers_mut().insert(
        "x-framerize-session",
        chat_session.parse().unwrap_or(axum::http::HeaderValue::from_static("")),
    );
    Ok(resp)
}

fn anthro_model_for_log(dialect: ClientDialect, payload: &Value) -> String {
    let _ = dialect;
    convert::to_openrouter_id(payload["model"].as_str().unwrap_or_default())
}

/// Anthropic count_tokens shape; character-based estimate (≈4 chars/token) —
/// the same fallback Claude Code applies when the endpoint is absent.
async fn count_tokens(Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
    let chars: usize = body["messages"]
        .as_array()
        .map(|ms| {
            ms.iter()
                .map(|m| {
                    m["content"]
                        .as_str()
                        .map(|s| s.len())
                        .or_else(|| {
                            m["content"].as_array().map(|bs| {
                                bs.iter()
                                    .map(|b| b["text"].as_str().map(|s| s.len()).unwrap_or(0))
                                    .sum()
                            })
                        })
                        .unwrap_or(0)
                })
                .sum::<usize>()
        })
        .unwrap_or(0)
        + body["system"].as_str().map(|s| s.len()).unwrap_or(0);
    Json(serde_json::json!({ "input_tokens": (chars / 4).max(1) }))
}

/// Paid-path wrappers: the metered streaming passthroughs, mounted at /paid/*.
async fn anthropic_messages_paid(
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

async fn openai_responses_paid(
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

async fn openai_chat_paid(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
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
    // Mint + signin happen INSIDE the account's browser: api.framer.com
    // validates request/environment consistency (Origin/UA/TLS vs the Castle
    // token's fingerprint) and 400s the bare-HTTP-client variant — observed.
    use framer_core::observability as obs;
    let t0 = std::time::Instant::now();
    let req_id = uuid::Uuid::new_v4().to_string();
    let r = s.browser.auth_begin(&body.account_id, &body.email).await;
    let mut ev = obs::Event::new("login.begin")
        .request_id(&req_id)
        .account(&body.account_id)
        .field("email", obs::mask_email(&body.email))
        .duration_ms(obs::elapsed_ms(t0));
    ev = match &r {
        Ok(()) => ev.outcome("ok"),
        Err(e) => ev.error(e),
    };
    ev.emit();
    r.map_err(ApiError)?;
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
    use framer_core::observability as obs;
    let t0 = std::time::Instant::now();
    let req_id = uuid::Uuid::new_v4().to_string();
    let r = s
        .browser
        .auth_complete(&body.account_id, &body.email_token)
        .await;
    let mut ev = obs::Event::new("login.complete")
        .request_id(&req_id)
        .account(&body.account_id)
        .duration_ms(obs::elapsed_ms(t0));
    ev = match &r {
        Ok(_) => ev.outcome("ok"),
        Err(e) => ev.error(e),
    };
    ev.emit();
    let (cookie_header, me) = r.map_err(ApiError)?;
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

    // Kick a background engine pre-warm so the first inference is instant.
    {
        let browser = s.browser.clone();
        let id = body.account_id;
        tokio::spawn(async move {
            let t0 = std::time::Instant::now();
            let r = browser.castle_token(&id).await;
            framer_core::observability::Event::new("account.prewarm")
                .account(&id)
                .duration_ms(framer_core::observability::elapsed_ms(t0))
                .outcome(if r.is_ok() { "ok" } else { "error" })
                .emit();
        });
    }
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
    // The agent route (/ai/v3/openai/responses/) takes UNPREFIXED model ids —
    // "openai/gpt-4.1" gets 400 "model pricing is not configured" (observed).
    // The chat/compat routes are the prefixed ones; strip here.
    let model = body
        .model
        .map(|m| m.rsplit_once('/').map(|(_, id)| id.to_string()).unwrap_or(m));
    let req = InferenceRequest {
        prompt: body.prompt,
        model,
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
