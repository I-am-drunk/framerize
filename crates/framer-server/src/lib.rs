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
    /// Paces requests against Framer's per-workspace concurrency cap so a large
    /// fleet queues instead of collecting 429s (framer_core::gate).
    pub gate: Arc<framer_core::gate::ConcurrencyGate>,
    /// Environment-level Castle-wave guard: pauses ALL traffic when refusals
    /// cluster across accounts (framer_core::wave).
    pub wave: Arc<framer_core::wave::WaveGuard>,
}

/// Build the router: REST under `/api`, compat under `/anthropic` + `/openai`,
/// everything else served from `web_dir`.
/// Abort a spawned chain when the response stream is dropped (client
/// disconnect/cancel). Dropping a bare JoinHandle DETACHES the task — the
/// chain kept minting Castle tokens and burning device velocity budget on
/// work nobody was reading ("zombie chains", 2026-10-05: the user's cancelled
/// T3 sessions kept consuming concurrency + velocity, and later requests
/// errored). This guard aborts the task on drop.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl<T> std::ops::Deref for AbortOnDrop<T> {
    type Target = tokio::task::JoinHandle<T>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T> std::ops::DerefMut for AbortOnDrop<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl<T> std::future::Future for AbortOnDrop<T> {
    type Output = Result<T, tokio::task::JoinError>;
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.0).poll(cx)
    }
}

pub fn router(state: AppState, web_dir: &str) -> Router {
    let api = Router::new()
        .route("/health", get(|| async { "ok" }))
        // Claude Code connection-warming probe (gateway protocol: safe to
        // answer or reject; we answer 200).
        .route("/hello", get(|| async { "ok" }).head(|| async { "ok" }))
        .route("/accounts", get(accounts))
        .route("/pool", get(pool_shape))
        .route("/accounts/remove", post(remove_account))
        .route("/accounts/dedupe", post(dedupe_accounts))
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
        .layer(axum::middleware::from_fn(access_log))
}

/// HTTP access log: every request lands in the NDJSON event log as
/// `http.request` (method, path, status, duration). Exists because Claude
/// Code's "repeated API errors" once had NO daemon-side trace — handler-level
/// rejections (4xx) and client-visible 5xx left no record anywhere.
async fn access_log(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let t0 = std::time::Instant::now();
    let resp = next.run(req).await;
    let status = resp.status().as_u16();
    if path != "/api/health" {
        use framer_core::observability::Event;
        Event::new("http.request")
            .field("method", method)
            .field("path", path)
            .field("status", status)
            .duration_ms(framer_core::observability::elapsed_ms(t0))
            .outcome(if status < 400 { "ok" } else { "error" })
            .emit();
    }
    resp
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
    let chat_session = session_hdr.unwrap_or_else(|| framer_api::inference::mint_id_pub());

    // Framer caps concurrent AI requests PER WORKSPACE (429 "too many
    // concurrent AI requests for this workspace" -- measured at 100 parallel).
    // Hold a permit for the whole request so a large fleet queues rather than
    // failing; the gate adapts its limit from observed 429s.
    let gate_key = account
        .current_team
        .as_ref()
        .map(|t| t.0.to_string())
        .unwrap_or_else(|| "default".to_string());
    let _permit = s.gate.acquire(&gate_key).await;

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
        .await;

    let upstream = match upstream {
        Ok(u) => {
            s.gate.on_success(&gate_key).await;
            u
        }
        Err(e) => {
            // 429 means "slow down", never "this account is bad"
            // (AGENTS.md rule 3). Narrow the gate and surface the error; the
            // client's retry lands behind a smaller limit.
            if e.contains("retryable 429") {
                s.gate.on_overloaded(&gate_key).await;
            }
            return Err(ApiError(e));
        }
    };

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
/// Persist the health consequence of one attempt.
///
/// A 403 risk refusal cools the account down with exponential backoff so
/// rotation stops choosing it; any success clears that state. Never
/// quarantines (AGENTS.md rule 3) and never treats a 429 as account health --
/// that is a workspace-level signal handled by the concurrency gate.
async fn record_account_health(
    s: &AppState,
    account: &framer_core::account::ManagedAccount,
    result: &Result<framer_core::convert::AssembledResponse, String>,
) {
    let now = framer_core::rotation::now_ms();
    let mut next = account.clone();
    match result {
        Ok(_) => {
            s.wave.note_success();
            if next.consecutive_failures == 0 && next.cooling_until_ms.is_none() {
                return; // already clean; skip the write
            }
            next.note_success();
        }
        // EXACT match only. `contains` would catch an "upstream 403 on ..."
        // whose echoed provider body happened to mention the word, and cooling
        // an account for the upstream's policy decision is the 2026-10-04 bug
        // (classify.rs): 1,399 upstream refusals read as Castle refusals drove
        // 2 of 3 accounts to the 300s cooldown cap and starved rotation.
        Err(e) if e == "risk-refused" => {
            s.wave.note_refusal(&account.id.0.to_string());
            next.note_risk_refused(now);
            eprintln!(
                "[health] {}: risk-refused -> cooling {}ms (step {})",
                &next.id.0.to_string()[..8],
                framer_core::account::ManagedAccount::cooldown_backoff_ms(next.cooldown_step - 1),
                next.cooldown_step,
            );
        }
        // Upstream refusals are loud but NOT account health: the operator
        // needs to see them (they mean a bad model id, a region block, or a
        // content filter), and the account must stay in rotation.
        Err(e) if e.starts_with("upstream 403") => {
            eprintln!(
                "[upstream] {}: provider refused (NOT account health): {}",
                &account.id.0.to_string()[..8],
                &e[..e.len().min(240)],
            );
            return;
        }
        Err(_) => return, // transport/429/other: not account health
    }
    if let Err(e) = s.state.upsert(&next).await {
        eprintln!("[health] upsert failed: {e}");
    }
}

async fn free_complete(
    s: AppState,
    headers: &axum::http::HeaderMap,
    dialect: ClientDialect,
    payload: serde_json::Value,
) -> Result<axum::response::Response, ApiError> {
    let client_model = payload["model"].as_str().unwrap_or_default().to_string();
    // Honest model routing (2026-10-05, user rule: never substitute silently).
    // gpt-6.x are direct-route models (OpenAI Responses, metered): Codex and
    // other Responses clients pass through natively. Genuinely unavailable
    // models get a clear error — the user picks what to do, not us.
    match convert::model_route(&client_model) {
        convert::ModelRoute::DirectResponses => {
            if dialect == ClientDialect::OpenAiResponses {
                return compat_forward(
                    s,
                    "openai-responses",
                    hdr(headers, "x-framerize-account"),
                    hdr(headers, "x-framerize-session"),
                    payload,
                )
                .await;
            }
            return Err(ApiError(format!(
                "model {client_model} is a direct-route model (OpenAI Responses API, metered) — call /openai/v1/responses for it; it is not on the free passthrough roster"
            )));
        }
        convert::ModelRoute::Unavailable(reason) => {
            return Err(ApiError(format!(
                "model {client_model} is not available on this gateway: {reason}"
            )));
        }
        convert::ModelRoute::FreePassthrough => {}
    }
    let wants_stream = payload["stream"].as_bool().unwrap_or(false);
    let include_usage = payload["stream_options"]["include_usage"].as_bool().unwrap_or(false);
    let max_tokens = payload["max_tokens"].as_u64()
        .or_else(|| payload["max_completion_tokens"].as_u64())
        .or_else(|| payload["max_output_tokens"].as_u64())
        .unwrap_or(4096) as u32;

    // Client request → faithful Anthropic Messages payload (model normalized).
    let anthro = convert::to_anthropic_request(dialect, &payload).map_err(ApiError)?;

    if let Some(remaining) = s.wave.check() {
        return Err(ApiError(format!(
            "environment cooling: Castle is refusing this egress right now — all traffic paused for {}s (retry then). Zero requests are being sent so the risk score can decay.",
            remaining.as_secs()
        )));
    }
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
        hdr(headers, "x-framerize-session").unwrap_or_else(|| framer_api::inference::mint_id_pub());

    // Framer caps concurrent AI requests PER WORKSPACE (429 "too many
    // concurrent AI requests for this workspace" -- measured: 64 parallel is
    // clean, 100 yields 64 of them 429). Hold a permit for the whole chain so a
    // large fleet QUEUES instead of failing. Bound to the function body, so it
    // releases on every exit path including the streaming spawn below.
    let gate_key = account
        .current_team
        .as_ref()
        .map(|t| t.0.to_string())
        .unwrap_or_else(|| "default".to_string());
    let _permit = s.gate.acquire(&gate_key).await;

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
        // Move the gate permit into the task: the chain outlives this function
        // body, and releasing at function exit would uncap concurrency for
        // every streaming client (which is all of Claude Code).
        let permit = _permit;
        let handle = AbortOnDrop(tokio::spawn(async move {
            let _permit = permit;
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
        }));

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
                            Ok(Err(e)) => convert::error_frames(dialect, &e),
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
    // A 429 is pure back-pressure: the request is valid and no work was done,
    // so retry it behind the now-narrower gate instead of failing the caller.
    // Without this the gate only protects LATER requests -- the ones already
    // in flight when the cap was hit still die (measured: 100 parallel ->
    // 37/100 with a gate that correctly walked 48->24->12->6).
    let mut result;
    let mut attempt = 0u32;
    loop {
        // Re-acquire per attempt so a retry waits for the narrowed limit.
        let permit = if attempt == 0 { None } else { Some(s.gate.acquire(&gate_key).await) };
        result = s
            .client
            .freechat_long(
                &session,
                s.browser.as_ref(),
                &account.id,
                &project,
                anthro.clone(),
                &cfg,
                &chat_session,
                None,
            )
            .await;
        drop(permit);
        // ONLY 429 is retryable here. A 403 risk-refusal must NOT be retried:
        // every attempt mints another Castle token from the same device, and
        // repeated minting from a device that is already being refused is
        // exactly the pattern the risk engine punishes
        // (reference/analysis/framer-castle-token.md). Retrying trades a
        // transient refusal for a sticky one. Castle pressure is prevented
        // upstream by the per-account mint gate, not recovered here.
        let overloaded = matches!(&result, Err(e)
            if e.contains("retryable 429") || e.contains("too many concurrent"));
        if !overloaded || attempt >= 5 {
            break;
        }
        s.gate.on_overloaded(&gate_key).await;
        attempt += 1;
        // Jittered backoff keyed off the attempt; the gate is already narrower.
        let backoff = 250u64 * (1 << attempt.min(4)) + u64::from(rand_u32() % 250);
        tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
    }

    // Feed the outcome back to the gate so the limit tracks reality: 429 means
    // "slow down", never "this account is bad" (AGENTS.md rule 3).
    match &result {
        Ok(_) => s.gate.on_success(&gate_key).await,
        Err(e) if e.contains("retryable 429") || e.contains("too many concurrent") => {
            s.gate.on_overloaded(&gate_key).await
        }
        Err(_) => {}
    }

    // Record ACCOUNT health. This route previously recorded none at all, so a
    // Castle-poisoned account stayed state=ready with failures=0 forever and
    // rotation kept feeding it: measured 2026-10-04, one account served 69
    // consecutive posts at 100% 403 while the gate sat at its ceiling of 64.
    // The gate was never the problem; the missing feedback loop was.
    record_account_health(&s, &account, &result).await;

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

/// Team ids already held by OTHER accounts. `exclude` is the account being
/// admitted, so a re-login keeps the workspace it already owns.
async fn claimed_workspaces(
    s: &AppState,
    exclude: AccountId,
) -> Result<std::collections::BTreeSet<String>, ApiError> {
    let pool = s.state.list().await.map_err(ApiError)?;
    Ok(pool
        .iter()
        .filter(|a| a.id != exclude)
        .filter_map(|a| a.current_team.as_ref().map(|t| t.0.clone()))
        .collect())
}

#[derive(Deserialize)]
struct RemoveBody {
    account_id: AccountId,
    /// Also drop the browser profile (cookies/storage/fingerprint). Off by
    /// default: a removed record can be re-imported, and wiping the profile
    /// regenerates the Castle device id, which is the pattern risk engines
    /// punish (reference/analysis/framer-castle-token.md).
    #[serde(default)]
    wipe_profile: bool,
}

/// Remove one account record.
async fn remove_account(
    State(s): State<AppState>,
    Json(body): Json<RemoveBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let existing = s.state.get(body.account_id).await.map_err(ApiError)?;
    let Some(acc) = existing else {
        return Err(ApiError("account not found".into()));
    };
    if body.wipe_profile {
        // Best effort: a missing browser profile must not block the removal.
        let _ = s.browser.rotate_identity(&body.account_id).await;
    }
    s.state.remove(body.account_id).await.map_err(ApiError)?;
    Ok(Json(serde_json::json!({
        "status": "removed",
        "account": acc.email.unwrap_or(acc.alias),
    })))
}

/// Find records that are genuinely redundant.
///
/// NOT "same login + same workspace". That pair shares a credit ledger and a
/// 429 budget, but each record still has its OWN Castle device id, and
/// spreading mints across devices is what stopped the 403 storm
/// (commit adfd926). Removing a second record on one team would halve that
/// protection while gaining nothing -- a dry run on 2026-10-04 proposed
/// deleting `probe`, which was carrying 50% of live traffic.
///
/// Genuinely redundant means one of:
///   * CLONE -- shares a castle_device_id with another record, so it does not
///     even add device diversity; or
///   * DEAD -- no project AND no credits AND no session, i.e. nothing to
///     recover. "No project" ALONE is not dead: a freshly added account has no
///     project yet while holding its full credit grant. Deleting on that alone
///     destroyed a 1000-credit account on 2026-10-04 (recovered from its
///     surviving session file). An unprovisioned account needs a project, not
///     a grave.
///
/// Everything else is kept, including same-login siblings.
async fn dedupe_accounts(
    State(s): State<AppState>,
    Json(body): Json<DedupeBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let pool = s.state.list().await.map_err(ApiError)?;
    let now = framer_core::rotation::now_ms();

    // Device clones: keep the first record per device id, flag later ones.
    let mut seen_devices: std::collections::BTreeSet<String> = Default::default();
    let mut doomed: Vec<(ManagedAccount, &'static str)> = Vec::new();
    let mut keeping = Vec::new();
    let mut needs_project: Vec<serde_json::Value> = Vec::new();

    // Stable order so the same record survives across repeated runs: usable
    // first, then one with credits, then fewest failures.
    let mut ordered = pool;
    ordered.sort_by(|a, b| {
        b.is_available_at(now)
            .cmp(&a.is_available_at(now))
            .then(
                b.credit
                    .as_ref()
                    .map(|c| c.available)
                    .unwrap_or(0.0)
                    .total_cmp(&a.credit.as_ref().map(|c| c.available).unwrap_or(0.0)),
            )
            .then(a.consecutive_failures.cmp(&b.consecutive_failures))
    });

    for a in ordered {
        let name = a.email.clone().unwrap_or_else(|| a.alias.clone());
        // Device clones first: that judgement needs no external state.
        if !seen_devices.insert(a.castle_device_id.clone()) {
            doomed.push((a, "shares a Castle device id with another record"));
            continue;
        }
        // Dead = nothing of value to recover. Credits or a live session make an
        // account worth provisioning rather than deleting.
        let has_credits = a.credit.as_ref().is_some_and(|c| c.available > 0.0);
        let has_session = s
            .sessions
            .load(&a.framer_user_id)
            .await
            .map_err(ApiError)?
            .is_some();
        if a.current_project.is_none() && !has_credits && !has_session {
            doomed.push((a, "no project, no credits, no session — nothing to recover"));
            continue;
        }
        if a.current_project.is_none() {
            // Kept, but flagged: it cannot serve traffic until it has a project.
            needs_project.push(serde_json::json!({
                "name": name,
                "credits": a.credit.as_ref().map(|c| c.available),
                "has_session": has_session,
                "reason": "no project yet — provision it, do not delete it",
            }));
        }
        keeping.push(serde_json::json!({
            "name": name,
            "team": a.current_team.as_ref().map(|t| t.0[..8.min(t.0.len())].to_string()),
            "device": a.castle_device_id[..8].to_string(),
        }));
    }

    let mut removed = Vec::new();
    for (dup, why) in doomed {
        let name = dup.email.clone().unwrap_or_else(|| dup.alias.clone());
        if body.apply {
            s.state.remove(dup.id).await.map_err(ApiError)?;
        }
        removed.push(serde_json::json!({ "name": name, "reason": why }));
    }

    Ok(Json(serde_json::json!({
        "applied": body.apply,
        "keeping": keeping,
        "needs_project": needs_project,
        "removing": removed,
        "note": "Same-login siblings are KEPT: each has its own Castle device id,                  and spreading mints across devices is what stops 403 storms. Only                  dead records (no project) and device clones are redundant.",
    })))
}

#[derive(Deserialize)]
struct DedupeBody {
    /// Dry run unless true, so the operator sees the plan first.
    #[serde(default)]
    apply: bool,
}

async fn accounts(State(s): State<AppState>) -> Result<Json<Vec<ManagedAccount>>, ApiError> {
    Ok(Json(s.state.list().await.map_err(ApiError)?))
}

/// Pool shape: what adding another account would actually buy.
///
/// Spreading load helps only along the axes that are genuinely separate.
/// Credits are WORKSPACE-scoped (reference/analysis/framer-credits.md: the
/// ledger is keyed by teamId) and so is the 429 concurrency cap, while Castle
/// risk is per DEVICE. So three records on one team share one credit pool and
/// one concurrency budget, and only spread device risk -- which looks like
/// capacity but isn't. This endpoint makes that visible instead of inferred.
async fn pool_shape(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let pool = s.state.list().await.map_err(ApiError)?;
    let now = framer_core::rotation::now_ms();

    let mut teams: std::collections::BTreeMap<String, Vec<&ManagedAccount>> = Default::default();
    for a in &pool {
        let key = a.current_team.as_ref().map(|t| t.0.clone()).unwrap_or_else(|| "none".into());
        teams.entry(key).or_default().push(a);
    }
    let users: std::collections::BTreeSet<&str> =
        pool.iter().map(|a| a.framer_user_id.0.as_str()).collect();
    let devices: std::collections::BTreeSet<&str> =
        pool.iter().map(|a| a.castle_device_id.as_str()).collect();

    let workspaces: Vec<serde_json::Value> = teams
        .iter()
        .map(|(team, accounts)| {
            serde_json::json!({
                "team": team,
                "accounts": accounts.iter().map(|a| {
                    serde_json::json!({
                        "name": a.email.clone().unwrap_or_else(|| a.alias.clone()),
                        "state": format!("{:?}", a.state),
                        "available": a.is_available_at(now),
                        "cooling_ms_left": a.cooling_until_ms
                            .map(|u| u.saturating_sub(now)).filter(|m| *m > 0),
                    })
                }).collect::<Vec<_>>(),
                "credits": accounts.iter().find_map(|a| a.credit.as_ref().map(|c| c.available)),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "accounts": pool.len(),
        "available_now": pool.iter().filter(|a| a.is_available_at(now)).count(),
        // These three are the levers, in descending order of effect.
        "distinct_workspaces": teams.len(),
        "distinct_logins": users.len(),
        "distinct_devices": devices.len(),
        "workspaces": workspaces,
        "note": "Credits and the 429 concurrency cap are per WORKSPACE; Castle risk is per DEVICE. Accounts sharing a team share credits and one concurrency budget.",
    })))
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
    /// Login email. Stored (normalized) as the human-readable identity and the
    /// dedupe key, so the same account cannot be added twice by accident.
    email: Option<String>,
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
            email: None,
            framer_user_id: session.active_user.clone(),
            castle_device_id: uuid::Uuid::new_v4().to_string(),
            proxy: None,
            current_team: None,
            current_project: None,
            state: framer_core::account::AccountState::Ready,
            credit: None,
            consecutive_failures: 0,
            cooldown_step: 0,
            cooling_until_ms: None,
        });
    // Dedupe by email BEFORE anything else: adding the same account twice
    // splits its credit ledger across two records and makes rotation think it
    // has more capacity than it does.
    if let Some(raw) = body.email.as_deref() {
        let email = framer_core::account::normalize_email(raw);
        let pool = s.state.list().await.map_err(ApiError)?;
        if let Some(dup) = pool
            .iter()
            .find(|a| a.id != account.id && a.email.as_deref() == Some(email.as_str()))
        {
            return Err(ApiError(format!(
                "{email} is already added as account {} — not adding it twice",
                dup.alias,
            )));
        }
        // The email is the operator-facing name; a uuid is not.
        if body.alias.is_none() {
            account.alias = email.clone();
        }
        account.email = Some(email);
    }
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

    // Admission: scan EVERY workspace concurrently and refuse the account if
    // none has credits. An account with no credited workspace cannot serve a
    // single request, so admitting it just feeds rotation guaranteed failures.
    // Parked as Exhausted (not deleted) so the operator can see why, and so it
    // becomes usable the moment credits appear.
    let deps = Deps {
        client: s.client.as_ref(),
        browser: s.browser.as_ref(),
        sessions: s.sessions.as_ref(),
        state: s.state.as_ref(),
    };
    let claimed = claimed_workspaces(&s, account.id).await?;
    let admission =
        framer_core::credits::admit(&deps, &mut account, &session, &claimed).await;
    if let Err(e) = admission {
        account.state = framer_core::account::AccountState::Exhausted;
        s.state.upsert(&account).await.map_err(ApiError)?;
        return Err(ApiError(e.to_string()));
    }
    s.state.upsert(&account).await.map_err(ApiError)?;

    Ok(Json(serde_json::json!({
        "status": "signed-in",
        "team": account.current_team.as_ref().map(|t| t.0.clone()),
        "credits": account.credit.as_ref().map(|c| c.available),
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
    /// Login email. Same role as in complete_login: operator-facing identity
    /// and dedupe key.
    email: Option<String>,
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
            email: None,
            framer_user_id: session.active_user.clone(),
            castle_device_id: uuid::Uuid::new_v4().to_string(),
            proxy: None,
            current_team: None,
            current_project: None,
            state: framer_core::account::AccountState::Ready,
            credit: None,
            consecutive_failures: 0,
            cooldown_step: 0,
            cooling_until_ms: None,
        });
    // Same dedupe + naming rule as complete_login: one email, one account.
    if let Some(raw) = body.email.as_deref() {
        let email = framer_core::account::normalize_email(raw);
        let pool = s.state.list().await.map_err(ApiError)?;
        if let Some(dup) = pool
            .iter()
            .find(|a| a.id != account.id && a.email.as_deref() == Some(email.as_str()))
        {
            return Err(ApiError(format!(
                "{email} is already added as account {} — not adding it twice",
                dup.alias,
            )));
        }
        if body.alias.is_none() {
            account.alias = email.clone();
        }
        account.email = Some(email);
    }
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

    // Admission (same rule as complete_login): an account with no credited
    // workspace cannot serve a request, so refuse it rather than let rotation
    // discover that one failure at a time.
    {
        let deps = Deps {
            client: s.client.as_ref(),
            browser: s.browser.as_ref(),
            sessions: s.sessions.as_ref(),
            state: s.state.as_ref(),
        };
        let claimed = claimed_workspaces(&s, account.id).await?;
        if let Err(e) =
            framer_core::credits::admit(&deps, &mut account, &session, &claimed).await
        {
            account.state = framer_core::account::AccountState::Exhausted;
            s.state.upsert(&account).await.map_err(ApiError)?;
            return Err(ApiError(e.to_string()));
        }
        // Adopt an existing project if the account has one. Every AI call needs
        // a projectId, so an imported account without one is admitted but
        // unusable -- and was previously mistaken for dead by the dedupe sweep.
        if account.current_project.is_none() {
            if let Some(team) = account.current_team.clone() {
                match s.client.team_projects(&session, &team).await {
                    Ok(projects) => {
                        if let Some(first) = projects.into_iter().next() {
                            eprintln!(
                                "[import] {}: adopted project {}",
                                account.alias, first.0
                            );
                            account.current_project = Some(first);
                        }
                    }
                    // Listing is best-effort: a dashboard shape change must not
                    // fail an otherwise good import.
                    Err(e) => eprintln!("[import] project listing failed: {e}"),
                }
            }
        }
        s.state.upsert(&account).await.map_err(ApiError)?;
    }

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
