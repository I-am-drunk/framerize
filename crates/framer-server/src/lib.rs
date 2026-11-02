//! REST/SSE routes, account admission and compatibility streaming.
//! Shared resource ownership lives in `state`; handlers retain those owners
//! through response delivery and independently observed external cleanup.

use axum::extract::{rejection::JsonRejection, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use framer_core::account::ManagedAccount;
use framer_core::convert::{self, ClientDialect};
use framer_core::error::AccountCooldown;
use framer_core::ids::AccountId;
use framer_core::pipeline::{self, Deps, InferenceOutcome, InferenceRequest};
#[cfg(test)]
use framer_core::port::{BrowserBridge, FramerClient, SessionStore, StateStore};
use serde::Deserialize;
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::services::ServeDir;

pub mod operation_ownership;
mod state;
pub use state::{AccountRuntime, AppState};

#[cfg(test)]
mod runtime_ownership_tests;

mod serving_accounts;
mod account_scope_publication;
mod account_wait;
mod pool_config;
mod pool_control;
mod request_token_budget;
#[cfg(test)]
mod responses_default_budget_tests;
mod request_body_object;
mod memory_admission;
mod model_discovery;
mod responses_aggregate;
mod response_output;
pub use serving_accounts::{ServingAccountPolicy, SERVING_ACCOUNTS_ENV};
pub use pool_config::{PoolConfig, EligibleAccounts, AllAccounts, POOL_SETTING_KEY};
pub use pool_control::prewarm_accounts;
use pool_control::RequestResidency;

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

#[cfg(test)]
#[path = "stream_ownership_tests.rs"]
mod stream_ownership_tests;

#[cfg(test)]
mod permit_lifetime_tests;

#[cfg(test)]
mod upstream_health_tests;
#[cfg(test)]
mod readiness_tests;

#[cfg(test)]
mod free_responses_context_tests;

#[cfg(test)]
mod account_registration_persistence_tests;

#[cfg(test)]
mod account_registration_concurrency_tests;

/// Keep the producer owned by its response body, including before the first poll.
fn proxy_response_stream(
    rx: tokio::sync::mpsc::Receiver<Result<framer_core::port::ProxyResponse, String>>,
    owner: AbortOnDrop<()>,
    dialect: ClientDialect,
) -> impl futures::Stream<Item = Result<axum::body::Bytes, String>> {
    futures::stream::unfold(
        (rx, None::<framer_core::port::ProxyResponse>, false, owner),
        move |(mut rx, mut up, drained, mut owner)| async move {
            if drained { let _ = (&mut owner).await; return None; }
            loop {
                if let Some(us) = &mut up {
                    // upstream arrived: yield its body chunks until exhausted
                    match futures::StreamExt::next(&mut us.stream).await {
                        Some(Ok(b)) if !b.is_empty() => {
                            return Some((Ok::<axum::body::Bytes, String>(b), (rx, up, drained, owner)));
                        }
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => return Some((Err(e), (rx, up, drained, owner))),
                        None => { let _ = (&mut owner).await; return None; },
                    }
                }
                tokio::select! {
                    r = rx.recv() => {
                        match r {
                            Some(Ok(us)) => { up = Some(us); continue; }
                            Some(Err(e)) => {
                                // Headers were sent. Clients need a terminal error event.
                                let msg = convert::error_frames(dialect, &e).join("\n\n") + "\n\n";
                                return Some((Ok(axum::body::Bytes::from(msg)), (rx, None, true, owner)));
                            }
                            None => { let _ = (&mut owner).await; return None; },
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(8)) => {
                        return Some((Ok::<axum::body::Bytes, String>(axum::body::Bytes::from_static(b": ping\n\n")), (rx, up, drained, owner)));
                    }
                }
            }
        },
    )
}

// Preserve raw/paid cancellation: only actual EOF can confirm this body.
// An early drop closes the signal without a verdict; an error keeps it false.
fn proxy_body<S>(stream: S, cleanup: pool_control::CleanupEvidence)
    -> (axum::body::Body, operation_ownership::ResponseCompletion)
where S: futures::Stream<Item = Result<axum::body::Bytes, String>> + Send + 'static {
    use futures::StreamExt;
    let (done, receiver) = tokio::sync::oneshot::channel();
    let stream = futures::stream::unfold((Box::pin(stream), done, cleanup),
        |(mut stream, done, cleanup)| async move {
            match stream.next().await {
                Some(chunk) => {
                    if chunk.is_err() { cleanup.include(false); }
                    Some((chunk, (stream, done, cleanup)))
                }
                None => {
                    drop(stream);
                    let _ = done.send(cleanup.confirmed());
                    None
                }
            }
        });
    (axum::body::Body::from_stream(stream),
        operation_ownership::ResponseCompletion::from_receiver(receiver))
}

pub fn router(state: AppState, web_dir: &str) -> Router {
    router_with_request_layer(state, web_dir, |router| router)
}

async fn health(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    health_response(state.operation_ownership.as_deref())
}

async fn readiness(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    use axum::response::IntoResponse;
    let check = async {
        let accounts = state.state.list().await?;
        let ids: Vec<_> = accounts.into_iter().map(|account| account.id).collect();
        state.browser.check_account_history(&ids).await
    };
    let admission = state.operation_ownership.as_ref().and_then(|scope|
        scope.admission_status().err().map(|error| scope.admission_message(error)));
    let problem = if let Some(cause) = admission {
        Some(("operation-admission-unavailable", cause))
    } else {
        match tokio::time::timeout(std::time::Duration::from_secs(5), check).await {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(("account-history-unavailable",
                framer_core::error::safe_failure_summary(&error))),
            Err(_) => Some(("account-history-unavailable",
                "account history validation did not finish within 5 seconds".into())),
        }
    };
    let ready = problem.is_none();
    let (reason, cause) = problem.map_or(("ready", None), |(reason, cause)| (reason, Some(cause)));
    let response = (if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE },
        [("x-framerize-pid", std::process::id().to_string())],
        Json(serde_json::json!({"ready": ready,
            "reason": reason, "cause": cause}))).into_response();
    // Discarding this completed diagnostic cannot leave inference work running.
    operation_ownership::completed_response(response, true)
}

fn health_response(ownership: Option<&operation_ownership::ServingOwnership>)
    -> impl axum::response::IntoResponse {
    let failure = ownership.and_then(|scope|
        scope.admission_status().err().map(|error| scope.admission_message(error)));
    let ready = failure.is_none();
    (if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE },
        [("x-framerize-pid", std::process::id().to_string())],
        failure.unwrap_or_else(|| "ok".into()))
}

#[cfg(test)]
#[tokio::test]
async fn health_identity_keeps_the_existing_body() {
    use axum::response::IntoResponse;
    let response = health_response(None).into_response();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(response.headers()["x-framerize-pid"], std::process::id().to_string());
    let body = axum::body::to_bytes(response.into_body(), 16).await.unwrap();
    assert_eq!(body.as_ref(), b"ok");
}

async fn hello() -> axum::response::Response {
    // HEAD removes this body outside the ownership layer. No external work
    // exists here, so dropping these completed bytes must not fence admission.
    operation_ownership::completed_response(
        axum::response::IntoResponse::into_response("ok"), true)
}

// These local reads have finished before their assembled bodies can be
// discarded. Keep this proof separate from all mutation and inference routes.
async fn accounts_read(state: State<AppState>) -> axum::response::Response {
    operation_ownership::completed_response(
        axum::response::IntoResponse::into_response(accounts(state).await), true)
}

async fn pool_read(state: State<AppState>) -> axum::response::Response {
    operation_ownership::completed_response(
        axum::response::IntoResponse::into_response(pool_shape(state).await), true)
}

async fn pool_config_read(state: State<AppState>) -> axum::response::Response {
    operation_ownership::completed_response(
        axum::response::IntoResponse::into_response(pool_control::config(state).await), true)
}

// Extraction has finished before this branch returns. A successfully extracted
// body carries no completion promise about the handler that will consume it.
async fn json_request<T, F, R>(state: State<AppState>,
    body: Result<Json<T>, JsonRejection>,
    handler: impl FnOnce(State<AppState>, Json<T>) -> F) -> axum::response::Response
where F: std::future::Future<Output = R>, R: axum::response::IntoResponse {
    use axum::response::IntoResponse;
    match body {
        Ok(body) => handler(state, body).await.into_response(),
        Err(error) => operation_ownership::completed_response(error.into_response(), true),
    }
}

// Only ServeDir uses this wrapper. Release the local body before reporting its
// disposal; this is not evidence about an upstream stream or OS descriptor close.
struct LocalFileBody {
    body: Option<axum::body::Body>,
    completed: Option<tokio::sync::oneshot::Sender<bool>>,
}

impl LocalFileBody {
    fn finish(&mut self) {
        drop(self.body.take());
        if let Some(completed) = self.completed.take() { let _ = completed.send(true); }
    }
}

impl Drop for LocalFileBody {
    fn drop(&mut self) { self.finish(); }
}

impl http_body::Body for LocalFileBody {
    type Data = axum::body::Bytes;
    type Error = axum::Error;
    fn poll_frame(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>)
        -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let Some(body) = self.body.as_mut() else { return std::task::Poll::Ready(None); };
        let frame = http_body::Body::poll_frame(std::pin::Pin::new(body), cx);
        if matches!(&frame, std::task::Poll::Ready(None | Some(Err(_)))) { self.finish(); }
        frame
    }
    fn is_end_stream(&self) -> bool {
        self.body.as_ref().map_or(true, http_body::Body::is_end_stream)
    }
    fn size_hint(&self) -> http_body::SizeHint {
        self.body.as_ref().map_or_else(http_body::SizeHint::default, http_body::Body::size_hint)
    }
}

async fn local_file_response(request: axum::extract::Request,
    next: axum::middleware::Next) -> axum::response::Response {
    let (mut parts, body) = next.run(request).await.into_parts();
    let (completed, receiver) = tokio::sync::oneshot::channel();
    parts.extensions.insert(operation_ownership::ResponseCompletion::from_receiver(receiver));
    axum::response::Response::from_parts(parts, axum::body::Body::new(LocalFileBody {
        body: Some(body), completed: Some(completed),
    }))
}

/// Install application metadata inside the existing owned handler task.
pub fn router_with_request_layer(state: AppState, web_dir: &str,
    layer: impl FnOnce(Router) -> Router) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/ready", get(readiness))
        // Claude Code connection-warming probe (gateway protocol: safe to
        // answer or reject; we answer 200).
        .route("/hello", get(hello).head(hello))
        .route("/accounts", get(accounts_read).head(accounts_read))
        .route("/pool", get(pool_read).head(pool_read))
        .route("/pool/config", get(pool_config_read).head(pool_config_read)
            .post(|state, body| json_request(state, body, pool_control::configure_observed)))
        .route("/pool/activate", post(|state, body| json_request(state, body, pool_control::activate_observed)))
        .route("/pool/park", post(|state, body| json_request(state, body, pool_control::park_observed)))
        .route("/accounts/remove", post(|state, body| json_request(state, body, remove_account_observed)))
        .route("/accounts/dedupe", post(|state, body| json_request(state, body, dedupe_accounts_observed)))
        .route("/accounts/begin", post(|state, body| json_request(state, body, begin_login_observed)))
        .route("/accounts/complete", post(|state, body| json_request(state, body, complete_login_observed)))
        .route("/accounts/import", post(|state, body| json_request(state, body, import_account_observed)))
        .route("/run", post(run_request));
    let api = api.with_state(state.clone());

    // Compat surfaces: thin translating proxies onto Framer's native dialect
    // routes. See reference/analysis/compat-mapping.md.
    let anthropic = Router::new()
        .route("/v1/models", get(model_discovery::anthropic))
        .route("/v1/messages", post(anthropic_messages))
        // Claude Code gateway protocol: token counting is optional (the client
        // falls back to a character estimate when absent), but cheap to serve.
        .route("/v1/messages/count_tokens", post(count_tokens))
        // Warming probe (gateway protocol): clients hit {BASE_URL}/api/hello —
        // with base /anthropic that's /anthropic/api/hello, NOT /api/hello
        // (measured: 30× 404s in 2h from Claude Code warming).
        .route("/api/hello", get(hello).head(hello))
        .with_state(state.clone());
    let openai = Router::new()
        .route("/v1/models", get(model_discovery::openai))
        .route("/v1/responses", post(openai_responses))
        .route("/v1/chat/completions", post(openai_chat))
        .route("/api/hello", get(hello).head(hello))
        .with_state(state.clone());

    // Convention: the BARE compat routes are the free path (unmetered chain),
    // and /paid/* holds the metered streaming passthroughs. Claude Code/Codex
    // can't send custom headers, so defaulting free at the normal path means
    // zero client config beyond the base URL; x-framerize-mode: paid remains
    // as a header override for clients that can set it.
    let anthropic_paid = Router::new()
        .route("/v1/models", get(model_discovery::anthropic))
        .route("/v1/messages", post(anthropic_messages_paid))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .with_state(state.clone());
    let openai_paid = Router::new()
        .route("/v1/models", get(model_discovery::openai))
        .route("/v1/responses", post(openai_responses_paid))
        .route("/v1/chat/completions", post(openai_chat_paid))
        .with_state(state.clone());
    let paid = Router::new()
        .nest("/anthropic", anthropic_paid)
        .nest("/openai", openai_paid);

    let files = Router::new().fallback_service(ServeDir::new(web_dir));
    let files = if state.operation_ownership.is_some() {
        files.layer(axum::middleware::from_fn(local_file_response))
    } else { files };
    let router = Router::new()
        .nest("/api", api)
        .nest("/anthropic", anthropic)
        .nest("/openai", openai)
        .nest("/paid", paid)
        .fallback_service(files);
    layer(router.layer(axum::middleware::from_fn_with_state(
        Arc::new(memory_admission::Admission::default()), memory_admission::middleware)))
        .layer(axum::middleware::from_fn_with_state(
            state.operation_ownership.clone(), operation_ownership::middleware))
        // The outer access logger observes ownership refusals as well.
        .layer(axum::middleware::from_fn(access_log))
        // Claude Code sends multi-MB contexts (images, long histories) — axum's
        // 2MB default was 413ing real sessions (2026-10-05).
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
}

/// HTTP access log: every request lands in the NDJSON event log as
/// `http.request` (method, path, status, duration). Exists because Claude
/// Code's "repeated API errors" once had NO daemon-side trace — handler-level
/// rejections (4xx) and client-visible 5xx left no record anywhere.
async fn access_log(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    #[cfg(test)]
    let access_capture = req.extensions().get::<operation_ownership::AccessLogCapture>().cloned();
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let t0 = std::time::Instant::now();
    let resp = next.run(req).await;
    let status = resp.status().as_u16();
    if path != "/api/health" {
        #[cfg(test)]
        if let Some(capture) = access_capture { capture.0.lock().unwrap().push(status); }
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
    serve_with_shutdown(state, web_dir, addr, std::future::pending()).await
}

pub async fn serve_with_shutdown(
    state: AppState, web_dir: &str, addr: SocketAddr,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "framerize listening");
    axum::serve(listener, router(state, web_dir))
        .with_graceful_shutdown(shutdown).await
}

/// Shared compat forward: pick an eligible account, forward the native payload
/// through the matching gateway dialect, stream the upstream response back
/// untouched (same dialect the client sent — verified 2026-10-01).
///
/// Account continuity: clients may pin a conversation with `x-framerize-account`
/// + `x-framerize-session` headers; otherwise the least-failed eligible account
///   and a fresh chat session are used.
async fn compat_forward_headers(
    s: AppState,
    dialect: &'static str,
    headers: &axum::http::HeaderMap,
    payload: serde_json::Value,
) -> Result<axum::response::Response, ApiError> {
    let outcome = compat_forward_observed(s, dialect, hdr(headers, "x-framerize-account"),
        hdr(headers, "x-framerize-session"), payload).await;
    match outcome.result {
        Ok(response) => Ok(response),
        Err(error) => Ok(operation_ownership::completed_response(
            axum::response::IntoResponse::into_response(error), outcome.cleanup_confirmed)),
    }
}

async fn compat_forward(
    s: AppState,
    dialect: &'static str,
    account_hdr: Option<String>,
    session_hdr: Option<String>,
    payload: serde_json::Value,
) -> Result<axum::response::Response, ApiError> {
    compat_forward_observed(s, dialect, account_hdr, session_hdr, payload).await.result
}

async fn compat_forward_observed(
    s: AppState,
    dialect: &'static str,
    account_hdr: Option<String>,
    session_hdr: Option<String>,
    mut payload: serde_json::Value,
) -> pool_control::CleanupOutcome<axum::response::Response> {
    let cleanup = pool_control::CleanupEvidence::new();
    let result = async {
    let client_dialect = match dialect {
        "anthropic-messages" => ClientDialect::AnthropicMessages,
        "openai-responses" => ClientDialect::OpenAiResponses,
        _ => ClientDialect::OpenAiChat,
    };
    request_body_object::validate(&payload)?;
    // Long non-streaming generations hit idle cutoffs (observed ~21.6 s in one
    // environment; a ~60 s server cutoff is plausible). ALWAYS stream upstream
    // and aggregate for clients that asked for stream:false.
    let client_wants_stream = payload["stream"].as_bool().unwrap_or(false);
    if !client_wants_stream {
        payload["stream"] = serde_json::Value::Bool(true);
    }

    let pool = s.state.list().await.map_err(ApiError::Internal)?;
    let residency = cleanup.outcome(pool_control::select_account_observed(
        &s, pool, account_hdr.as_deref(), &[],
    ).await)?;
    let account = cleanup.admission(current_account(&s, residency.account()).await)?;
    let session = s
        .sessions
        .load(&account.framer_user_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::Internal("account has no session".into()))?;
    let project = account
        .current_project
        .clone()
        .ok_or_else(|| ApiError::Internal("account has no current project".into()))?;
    let chat_session = session_hdr.unwrap_or_else(|| stable_session(&account.id));

    // CREDIT-BOND GUARD (2026-10-05): metered (direct) routes spend the SAME
    // balance the free route needs to stay alive — a 0-credit workspace 402s
    // on EVERYTHING. A gpt-6.x codex session burned 1,400 credits across two
    // workspaces in one evening and killed both. Refuse metered calls below
    // the floor (env FRAMERIZE_CREDIT_FLOOR, default 25) so a workspace can
    // never be burned to death again; free models stay unlimited.
    if let Some(team) = account.current_team.as_ref() {
        let floor: f64 = std::env::var("FRAMERIZE_CREDIT_FLOOR")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(25.0);
        match cleanup.external(s.client.usage(&session, team).await) {
            Ok(snap) => {
                if snap.available < floor {
                    framer_core::observability::Event::new("credits.bond_block")
                        .account(&account.id)
                        .field("available", snap.available)
                        .field("floor", floor)
                        .outcome("error")
                        .emit();
                    return Err(ApiError::InsufficientCredits { available: snap.available, floor });
                }
                let _ = pipeline::update_account(s.state.as_ref(),
                    &s.serving_accounts.health_update, &account,
                    |current| current.credit = Some(snap)).await;
            }
            Err(e) => {
                // Ledger unreadable: allow if the cached snapshot is above the
                // floor, else block (fail-safe — the bond matters more than
                // one metered call).
                let cached = account.credit.as_ref().map(|c| c.available);
                if cached.map(|a| a < floor).unwrap_or(true) {
                    let balance = cached.map(|value| format!("{value:.2}"))
                        .unwrap_or_else(|| "unknown".into());
                    return Err(ApiError::ServingCapacity(format!(
                        "Cannot verify workspace credits because reading Framer's ledger failed: {}. The cached balance is {balance}; this metered route requires at least {floor:.2} credits. Restore ledger access before retrying.",
                        framer_core::error::safe_failure_summary(&e),
                    )));
                }
            }
        }
    }

    // Framer caps concurrent AI requests per (workspace × MODEL) — measured
    // 2026-10-05: ~6 in-flight per model, category-independent; 6 models × 6
    // = 36/36 clean while 12× one model = 6 pass / 6× 429. The gate keys
    // accordingly so load spread across the model roster never trips it.
    let gate_model = framer_core::convert::to_openrouter_id(
        payload["model"].as_str().unwrap_or("unknown"),
    );
    let gate_key = account
        .current_team
        .as_ref()
        .map(|t| format!("{}:{gate_model}", t.0))
        .unwrap_or_else(|| "default".to_string());
    let _permit = Arc::new(tokio::select! {
        biased;
        _ = operation_ownership::request_disconnected() => {
            return Err(ApiError::Internal("client cancelled".into()));
        }
        permit = s.gate.acquire(&gate_key) => permit,
    });
    cleanup.admission(current_account(&s, account.id).await)?;

    // STREAMING CLIENTS: return immediately with an SSE response; the upstream
    // call runs in a task while the body emits keepalive pings. Without this
    // the client sat with NO headers for the whole generation (the in-page
    // transport completes before it yields) and codex's idle timeout killed
    // every long turn — "We're currently experiencing high demand".
    if client_wants_stream {
        let s2 = s.clone();
        let account2 = account.clone();
        let session2 = session.clone();
        let project2 = project.clone();
        let chat2 = chat_session.clone();
        let dialect2 = dialect.to_string();
        let gate2 = s.gate.clone();
        let gate_key2 = gate_key.clone();
        let producer_permit = _permit.clone();
        let producer_residency = cleanup.outcome(pool_control::lease_account_observed(&s, account.id).await)?;
        cleanup.admission(current_account(&s, account.id).await)?;
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<framer_core::port::ProxyResponse, String>>(1);
        let producer_cleanup = cleanup.clone();
        let handle = AbortOnDrop(operation_ownership::spawn_with_cleanup(s.operation_ownership.as_ref(),
            framer_core::operation_gate::OperationKind::Inference, async move {
            let _producer_permit = producer_permit;
            let producer_residency = Arc::new(producer_residency);
            let r = match producer_cleanup.admission(current_account(&s2, account2.id).await) {
                Ok(_) => s2
                .client
                .proxy(&session2, s2.browser.as_ref(), &account2.id, &project2, &dialect2, payload, &chat2)
                .await,
                Err(error) => Err(chain_admission_error(error)),
            };
            let stream_cleanup = producer_cleanup.clone();
            let r = pool_control::observe_lease_result(&producer_residency, producer_cleanup.external(r)).map(|mut response| {
                use futures::StreamExt;
                response.stream = Box::pin(response.stream.map(move |chunk|
                    pool_control::observe_lease_result(&producer_residency, stream_cleanup.external(chunk))));
                response
            });
            match &r {
                Ok(_) => gate2.on_success(&gate_key2).await,
                Err(e) if framer_core::error::is_workspace_saturation(e) => {
                    gate2.on_overloaded(&gate_key2).await;
                    if let Some(team) = account2.current_team.as_ref() {
                        s2.serving_accounts.rotation().mark_saturated(&team.0, 45_000);
                    }
                }
                Err(_) => {}
            }
            let _ = tx.send(r).await;
            ((), producer_cleanup.confirmed())
        }).map_err(ApiError::operation_admission)?);
        let stream = retain_owner(proxy_response_stream(rx, handle, client_dialect), (residency, _permit));
        let (body, completion) = proxy_body(stream, cleanup.clone());
        let mut resp = axum::response::Response::new(body);
        resp.extensions_mut().insert(completion);
        resp.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
        resp.headers_mut().insert(
            "x-framerize-session",
            chat_session.parse().unwrap_or(axum::http::HeaderValue::from_static("")),
        );
        return Ok(resp);
    }

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

    let upstream = pool_control::observe_lease_result(&residency, cleanup.external(upstream));
    let upstream = match upstream {
        Ok(u) => {
            s.gate.on_success(&gate_key).await;
            u
        }
        Err(e) => {
            // 429 means "slow down", never "this account is bad"
            // (AGENTS.md rule 3). Narrow the gate and surface the error; the
            // client's retry lands behind a smaller limit.
            if framer_core::error::is_workspace_saturation(&e) {
                s.gate.on_overloaded(&gate_key).await;
                if let Some(team) = account.current_team.as_ref() {
                    s.serving_accounts.rotation().mark_saturated(&team.0, 45_000);
                }
            }
            let (status, body) = convert::error_json(client_dialect, &e);
            let response = axum::response::IntoResponse::into_response(ShapedApiError(
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), body));
            return Ok(operation_ownership::completed_response(response, cleanup.confirmed()));
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
            buf.extend_from_slice(&pool_control::observe_lease_result(&residency, cleanup.external(chunk))
                .map_err(ApiError::Internal)?);
        }
        let aggregated = match std::str::from_utf8(&buf) {
            Ok(body) => aggregate_sse(dialect, body),
            Err(_) if dialect == "openai-responses" => Err("Responses stream is not valid UTF-8"),
            Err(_) => aggregate_sse(dialect, &String::from_utf8_lossy(&buf)),
        };
        let (status, aggregated) = match aggregated {
            Ok(value) => (status, value),
            Err(message) => (
                if status.is_success() { StatusCode::BAD_GATEWAY } else { status },
                serde_json::json!({"error":{"code":"invalid_upstream_response","message":message}}),
            ),
        };
        let mut resp = axum::response::Response::new(axum::body::Body::from(
            serde_json::to_string(&aggregated).map_err(|e| ApiError::Internal(e.to_string()))?,
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
        return Ok(operation_ownership::completed_response(resp, cleanup.confirmed()));
    }

    // STREAMING HEARTBEAT (2026-10-05): the in-page transport returns the
    // upstream body only at COMPLETION, and the old code awaited it inside the
    // handler — the client got neither headers nor bytes for the whole
    // generation, so codex's app-server idle-timeout killed every long turn
    // (responseStreamDisconnected{httpStatusCode:null} → its generic
    // "We're currently experiencing high demand" message). Fix: return the
    // SSE response immediately, run the upstream call in a task, emit
    // keepalive comment pings during the wait, then stream the body.
    // Non-streaming client + non-SSE upstream (plain JSON): pass through.
    // (Streaming clients returned early above; SSE upstreams aggregate above.)
    use futures::StreamExt;
    let residency = Arc::new(residency);
    let observer = residency.clone();
    let stream = upstream.stream.map(move |chunk| {
        pool_control::observe_lease_result(&observer, chunk)
    });
    let (body, completion) = proxy_body(retain_owner(stream, (residency, _permit)), cleanup.clone());
    let mut resp = axum::response::Response::new(body);
    resp.extensions_mut().insert(completion);
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
    }.await;
    cleanup.finish(result)
}

/// Fold an upstream SSE stream into the dialect's non-streaming JSON response.
/// Verified event shapes in reference/analysis/verified-captures-2026-10-01.md.
fn retain_owner<S: futures::Stream, T>(stream: S, owner: T) -> impl futures::Stream<Item = S::Item> {
    use futures::StreamExt;
    futures::stream::unfold((Box::pin(stream), owner), |(mut stream, owner)| async move {
        stream.next().await.map(|item| (item, (stream, owner)))
    })
}

fn aggregate_sse(dialect: &str, body: &str) -> Result<serde_json::Value, &'static str> {
    if dialect == "openai-responses" { return responses_aggregate::parse(body); }
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

    Ok(match dialect {
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
    })
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
    payload: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<axum::response::Response, ApiError> {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(error) => {
            let response = axum::response::IntoResponse::into_response(error);
            return Ok(operation_ownership::completed_response(response, true));
        }
    };
    if hdr(&headers, "x-framerize-mode").as_deref() != Some("paid") {
        return free_complete(s, &headers, ClientDialect::AnthropicMessages, payload).await;
    }
    compat_forward_headers(
        s,
        "anthropic-messages",
        &headers,
        payload,
    )
    .await
}

async fn openai_responses(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    payload: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<axum::response::Response, ApiError> {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(error) => {
            let response = axum::response::IntoResponse::into_response(error);
            return Ok(operation_ownership::completed_response(response, true));
        }
    };
    if hdr(&headers, "x-framerize-mode").as_deref() != Some("paid") {
        return free_complete(s, &headers, ClientDialect::OpenAiResponses, payload).await;
    }
    compat_forward_headers(
        s,
        "openai-responses",
        &headers,
        payload,
    )
    .await
}

async fn openai_chat(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    payload: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<axum::response::Response, ApiError> {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(error) => {
            let response = axum::response::IntoResponse::into_response(error);
            return Ok(operation_ownership::completed_response(response, true));
        }
    };
    if hdr(&headers, "x-framerize-mode").as_deref() != Some("paid") {
        return free_complete(s, &headers, ClientDialect::OpenAiChat, payload).await;
    }
    compat_forward_headers(
        s,
        "openrouter-chat",
        &headers,
        payload,
    )
    .await
}


/// Pick this chain's workspace scope: round-robins the account's credited
/// workspaces (the per-workspace 429 cap is THE concurrency goose), preferring
/// non-saturated ones. One account = N workspace budgets (measured 2026-10-06).
fn pick_scope(s: &AppState, account: &ManagedAccount) -> Result<framer_core::account::WorkspaceScope, ApiError> {
    let scopes = account.all_scopes();
    if scopes.is_empty() {
        return Err(ApiError::Internal("account has no current project".into()));
    }
    if scopes.len() == 1 {
        return Ok(scopes.into_iter().next().unwrap());
    }
    let start = s.serving_accounts.rotation().next_scope_idx(&account.id.0.to_string(), scopes.len());
    for k in 0..scopes.len() {
        let sc = &scopes[(start + k) % scopes.len()];
        if !s.serving_accounts.rotation().is_saturated(&sc.team.0) {
            return Ok(sc.clone());
        }
    }
    Ok(scopes[start].clone())
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

/// Upstream refusal classes that mean this account is being refused. Distinct from
/// upstream 403s (provider policy — bad model id, region, content), which say
/// nothing about the account. Unwrap only the chain's known wrapper, then
/// match its exact sentinel. Provider text cannot contribute to a wave merely
/// because it ends with the same words.
fn is_castle_refusal(e: &str) -> bool {
    framer_core::error::is_security_refusal(e)
}

fn is_server_cooling(error: &str) -> bool {
    framer_core::error::chain_error_cause(error).strip_prefix("server-cooling:")
        .and_then(|seconds| seconds.parse::<u64>().ok()).is_some()
}

fn should_retry_account(_error: &str) -> bool {
    // Keep one account identity for the whole request, including failures.
    false
}

/// Pick a serving account excluding the ones already tried for this request.
/// Re-reads the pool so health marks written moments ago are honored.
async fn pick_fresh_account(
    s: &AppState,
    excluded: &[String],
) -> pool_control::AcquisitionOutcome<(ManagedAccount, framer_core::resident_pool::ResidentLease)> {
    let mut cleanup_confirmed = true;
    let result = async {
        let mut pool = s.state.list().await.map_err(ApiError::Internal)?;
        let candidates: Vec<_> = pool.iter().map(|account| account.id).collect();
        let mut skipped = excluded.to_vec();
        loop {
            // Each original candidate is admitted at most once. A refused
            // local admission has made no inference attempt on that account.
            let preferred = s.serving_accounts.preferred_idle(&pool)?;
            let mut selected_id = None;
            let acquired = s.serving_accounts.acquire_selected_observed(
                s.browser.as_ref(), || {
                    let account = s.serving_accounts.select_fresh(pool.clone(), &skipped)?;
                    selected_id = Some(account.id);
                    Ok(account.id)
                }, preferred, Some(s.state.as_ref()),
            ).await;
            if let Some(id) = selected_id { skipped.push(id.0.to_string()); }
            cleanup_confirmed &= acquired.cleanup_confirmed;
            let result = match acquired.result {
                Ok(lease) => current_account(s, lease.account()).await.map(|account| (account, lease)),
                Err(error) => Err(error),
            };
            match result {
                Err(ApiError::AccountCooling { .. } | ApiError::PoolBusy(_)
                    | ApiError::ServingCapacity(_)) if cleanup_confirmed && selected_id.is_some() => {},
                result => return result,
            }
            pool = s.state.list().await.map_err(ApiError::Internal)?;
            pool.retain(|account| candidates.contains(&account.id));
        }
    }.await;
    pool_control::AcquisitionOutcome { result, cleanup_confirmed }
}

async fn current_account(s: &AppState, id: AccountId) -> Result<ManagedAccount, ApiError> {
    let snapshot = s.serving_accounts.residents.snapshot()
        .map_err(|e| ApiError::PoolBusy(e.to_string()))?;
    if let Some(slot) = snapshot.residents.iter().find(|slot| slot.account == id
        && (slot.uncertain || slot.status == framer_core::resident_pool::ResidentStatus::Closing)) {
        let cause = slot.cleanup_cause.as_deref().unwrap_or(if slot.uncertain {
            "browser cleanup stopped without a retained failure cause"
        } else { "browser closure is still running" });
        return Err(ApiError::PoolBusy(format!("{cause}; the occupied browser slot cannot be reused")));
    }
    let account = s.state.get(id).await.map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::InvalidRequest("account not found".into()))?;
    if let Some(remaining) = s.wave.check() {
        return Err(ApiError::Cooling { retry_after_secs: remaining.as_secs().saturating_add(1) });
    }
    let now = framer_core::rotation::now_ms();
    if let Some(until) = account.cooling_until_ms.filter(|until| *until > now) {
        return Err(ApiError::AccountCooling {
            retry_after_secs: until.saturating_sub(now).div_ceil(1000),
            cause: account.cooldown_reason().to_owned(),
        });
    }
    if let Some(cause) = account.eligibility_problem() {
        return Err(ApiError::ServingCapacity(cause));
    }
    Ok(account)
}

fn chain_admission_error(error: ApiError) -> String {
    match error {
        error @ ApiError::InsufficientCredits { .. } => error.diagnostic(),
        ApiError::AccountCooling { retry_after_secs, cause } =>
            format!("account-cooling:{retry_after_secs}; {cause}"),
        ApiError::Cooling { retry_after_secs } => format!("server-cooling:{retry_after_secs}"),
        ApiError::InvalidRequest(message) | ApiError::PoolBusy(message)
        | ApiError::Internal(message) | ApiError::ServingCapacity(message) => message,
    }
}

fn error_excerpt(error: &str, max_bytes: usize) -> &str {
    let mut end = error.len().min(max_bytes);
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    &error[..end]
}

async fn record_account_health<T>(
    s: &AppState,
    account: &framer_core::account::ManagedAccount,
    result: &Result<T, String>,
) {
    let _update = s.serving_accounts.health_update.lock().await;
    let Ok(Some(current)) = s.state.get(account.id).await else { return; };
    if current.framer_user_id != account.framer_user_id { return; }
    let now = framer_core::rotation::now_ms();
    let mut next = current;
    match result {
        Ok(_) => {
            // A response admitted before a newer refusal cannot clear it.
            if next.cooldown_step != account.cooldown_step
                || next.cooling_until_ms != account.cooling_until_ms
                || next.cooldown_cause != account.cooldown_cause
                || next.consecutive_failures != account.consecutive_failures { return; }
            s.wave.note_success();
            if next.consecutive_failures == 0 && next.cooling_until_ms.is_none() {
                return; // already clean; skip the write
            }
            next.note_success();
        }
        Err(e) if AccountCooldown::from_error(e).is_some() => {
            let cooldown = AccountCooldown::from_error(e).expect("matched local cooldown");
            let until = now.saturating_add(cooldown.retry_after_secs.saturating_mul(1000));
            let cause = framer_core::error::chain_error_cause(e)
                .split_once("; ").map(|(_, cause)| cause);
            next.note_cooldown(until, cause);
            framer_core::observability::Event::new("account.local_cooldown")
                .account(&account.id)
                .field("retry_after_secs", cooldown.retry_after_secs)
                .emit();
        }
        // SUFFIX match: chains compose "chunk N retry died (fallback off):
        // risk-refused" — the sentinel is the tail, so the old EXACT match
        // never fired (measured 2026-10-11: conform served 20/20 refusals at
        // fail: 0). "upstream 403 ..." bodies are excluded by is_castle_refusal
        // (their policy refusal is not the account's health).
        Err(e) if is_castle_refusal(e) => {
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
                error_excerpt(e, 240),
            );
            return;
        }
        Err(_) => return, // transport/429/other: not account health
    }
    if let Err(e) = s.state.upsert(&next).await {
        eprintln!("[health] upsert failed: {e}");
    }
}

/// Stable chat session per account per ~5h window (invisibility, 2026-10-05):
/// clients that don't send x-framerize-session got a fresh session per
/// REQUEST — thousands of 1-turn sessions/day, an obvious anomaly. Real usage
/// is task-length conversations. Rotates the id every 5h; same 9-char base62
/// shape as the real client.
fn stable_session(account: &AccountId) -> String {
    let window = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / (5 * 3600))
        .unwrap_or(0) as u128;
    let mut n = window ^ (account.0.as_u128().wrapping_mul(0x9e3779b97f4a7c15));
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut out = String::with_capacity(9);
    for _ in 0..9 {
        out.push(ALPHA[(n % 62) as usize] as char);
        n /= 62;
    }
    out
}

async fn free_complete(
    s: AppState,
    headers: &axum::http::HeaderMap,
    dialect: ClientDialect,
    payload: serde_json::Value,
) -> Result<axum::response::Response, ApiError> {
    request_body_object::validate(&payload)
        .map_err(operation_ownership::completed_preflight)?;
    let mut payload = payload;
    // T3 ultrafast variants (compat/t3/ultrafast): a model id ending in
    // "-ultrafast" strips to the base model and rides upstream with
    // service_tier:"ultrafast" (OpenAI's lowest-latency tier — free on the
    // passthrough, verified delta 0.0; metered+premium on the direct route).
    let ultrafast = payload["model"].as_str().unwrap_or_default().ends_with("-ultrafast");
    if ultrafast {
        let base = payload["model"].as_str().unwrap_or_default().trim_end_matches("-ultrafast").to_string();
        // Tier by route (verified 2026-10-06): the OpenRouter passthroughs take
        // "ultrafast" (and fall back to priority internally per the docs); the
        // direct Responses API rejects it for sol ("Invalid service_tier
        // argument") — priority is the best tier it accepts.
        let tier = if convert::model_route(&base) == convert::ModelRoute::DirectResponses {
            "priority"
        } else {
            "ultrafast"
        };
        payload["model"] = serde_json::Value::String(base);
        payload["service_tier"] = serde_json::Value::String(tier.into());
    }
    let client_model = payload["model"].as_str().unwrap_or_default().to_string();
    // Codex "custom" tools (exec, apply_patch, …) ride upstream in an
    // {"input": …} wrapper; the render side unwraps them by name.
    let custom_tools = convert::custom_tool_names(&payload);
    // Honest model routing (2026-10-05, user rule: never substitute silently).
    // gpt-6.x are direct-route models (OpenAI Responses, metered): Codex and
    // other Responses clients pass through natively. Genuinely unavailable
    // models get a clear error — the user picks what to do, not us.
    match convert::model_route(&client_model) {
        convert::ModelRoute::DirectResponses => {
            // Metered route BLOCKED by default (user rule 2026-10-06: "we
            // should not be using the paid route"). gpt-6-sol/6.1-sol/luna/
            // terra only exist on the metered direct route, so they are
            // unavailable unless the operator explicitly opts into spend.
            if std::env::var("FRAMERIZE_ALLOW_METERED").map(|v| v == "1").unwrap_or(false) {
                if dialect == ClientDialect::OpenAiResponses {
                    return compat_forward_headers(
                        s,
                        "openai-responses",
                        headers,
                        payload,
                    )
                    .await;
                }
                return Err(operation_ownership::completed_preflight(ApiError::Internal(format!(
                    "model {client_model} is a direct-route model (OpenAI Responses API, metered) — call /openai/v1/responses for it"
                ))));
            }
            return Err(operation_ownership::completed_preflight(ApiError::Internal(format!(
                "model {client_model} is metered-only (direct route) and metered access is disabled on this gateway (set FRAMERIZE_ALLOW_METERED=1 to allow spend). Free alternatives: gpt-6-astra, gpt-4.1, kimi-k3, gemini-2.5-flash, and the Claude roster."
            ))));
        }
        convert::ModelRoute::Unavailable(reason) => {
            return Err(operation_ownership::completed_preflight(ApiError::Internal(format!(
                "model {client_model} is not available on this gateway: {reason}"
            ))));
        }
        convert::ModelRoute::FreePassthrough => {}
    }
    // The chain cannot resolve stored context; stripping its ID loses history.
    if dialect == ClientDialect::OpenAiResponses
        && payload.get("previous_response_id").is_some_and(|value| !value.is_null())
    {
        return Err(operation_ownership::completed_preflight(ApiError::InvalidRequest(
            "previous_response_id is unsupported on this route; send complete input history".into(),
        )));
    }
    let wants_stream = payload["stream"].as_bool().unwrap_or(false);
    let include_usage = payload["stream_options"]["include_usage"].as_bool().unwrap_or(false);
    let max_tokens = if dialect == ClientDialect::OpenAiResponses {
        request_token_budget::explicit_from_payload(&payload)
    } else {
        request_token_budget::from_payload(&payload).map(Some)
    }
        .map_err(|message| ApiError::InvalidRequest(message.into()))
        .map_err(operation_ownership::completed_preflight)?;

    // THE DIALECT-NATIVE RULE (docs/native-chains.md, 2026-10-06): OpenAI
    // clients never convert to Anthropic. Responses clients run the
    // responses-native chain verbatim; chat clients map thinly onto it
    // (chat→responses is OpenAI→OpenAI). Anthropic clients use the existing
    // anthro chain. The conversion that caused the "no tools" bug class is
    // dead for OpenAI clients.
    if dialect != ClientDialect::AnthropicMessages {
        return native_complete(s, headers, dialect, payload, wants_stream, include_usage, max_tokens).await;
    }
    let max_tokens = max_tokens.unwrap_or(4096);

    // Client request → faithful Anthropic Messages payload (model normalized).
    let anthro = convert::to_anthropic_request(dialect, &payload)
        .map_err(ApiError::Internal).map_err(operation_ownership::completed_preflight)?;

    if let Some(remaining) = s.wave.check() {
        return Err(operation_ownership::completed_preflight(ApiError::Cooling {
            retry_after_secs: remaining.as_secs().saturating_add(1),
        }));
    }
    if headers.get("x-framerize-account").is_some_and(|value| value.to_str().is_err()) {
        return Err(operation_ownership::completed_preflight(ApiError::InvalidRequest(
            "x-framerize-account must contain valid text".into(),
        )));
    }
    let requested_account = hdr(headers, "x-framerize-account");
    let allow_account_repick = requested_account.is_none();
    let gate_model = anthro["model"].as_str().unwrap_or("unknown").to_string();
    // A queued account can cool before its turn. Try at most two peers,
    // releasing this iteration's lease and permit before selecting another.
    let mut skipped = Vec::new();
    let (account, residency, mut session, mut scope, mut first_permit) = loop {
        let pool = s.state.list().await.map_err(ApiError::Internal)?;
        let acquired = pool_control::select_account_observed(
            &s, pool, requested_account.as_deref(), &skipped).await;
        let lease = match acquired.result {
            Ok(lease) => lease,
            Err(error) if skipped.is_empty() && acquired.cleanup_confirmed =>
                return Err(operation_ownership::completed_preflight(error)),
            Err(error) => return Ok(operation_ownership::completed_response(
                axum::response::IntoResponse::into_response(error), acquired.cleanup_confirmed)),
        };
        let selected_id = lease.account();
        let residency = RequestResidency::new(lease);
        let account = match current_account(&s, selected_id).await {
            Ok(account) => account,
            Err(ApiError::AccountCooling { .. }) if allow_account_repick && skipped.len() < 2 => {
                skipped.push(selected_id.0.to_string());
                continue;
            },
            Err(error @ (ApiError::Cooling { .. } | ApiError::AccountCooling { .. }
                | ApiError::ServingCapacity(_))) => {
                return Ok(operation_ownership::completed_response(
                    axum::response::IntoResponse::into_response(error), true));
            },
            Err(error) => return Err(error),
        };
        let session = s
            .sessions
            .load(&account.framer_user_id)
            .await
            .map_err(ApiError::Internal)?
            .ok_or_else(|| ApiError::Internal("account has no session".into()))?;
        let scope = pick_scope(&s, &account)?;

        // Per (workspace × MODEL) concurrency — measured 2026-10-05: ~6 in-flight
        // per model; 6 models × 6 = 36/36 clean. Keyed on the normalized upstream
        // id (client spellings collapse to the true bucket) AND the chain's
        // rotated workspace (one account now spans multiple workspace budgets).
        let gate_key = format!("{}:{gate_model}", scope.team.0);
        let first_permit = Some(tokio::select! {
            biased;
            _ = operation_ownership::request_disconnected() => {
                let (status, body) = convert::error_json(dialect, "client cancelled");
                let response = axum::response::IntoResponse::into_response(ShapedApiError(
                    StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), body,
                ));
                return Ok(operation_ownership::completed_response(response, true));
            }
            permit = s.gate.acquire(&gate_key) => permit,
        });
        match current_account(&s, account.id).await {
            Ok(_) => {},
            Err(ApiError::AccountCooling { .. }) if allow_account_repick && skipped.len() < 2 => {
                skipped.push(account.id.0.to_string());
                continue;
            },
            Err(error @ (ApiError::Cooling { .. } | ApiError::AccountCooling { .. }
                | ApiError::ServingCapacity(_))) => {
                return Ok(operation_ownership::completed_response(
                    axum::response::IntoResponse::into_response(error), true));
            },
            Err(error) => return Err(error),
        }

        break (account, residency, session, scope, first_permit);
    };
    let mut project = scope.project.clone();
    let mut scope_team = scope.team.clone();
    let mut chat_session = hdr(headers, "x-framerize-session")
        .unwrap_or_else(|| stable_session(&account.id));
    let mut gate_key = format!("{}:{gate_model}", scope_team.0);
    let mk_cfg = |team: &framer_core::ids::TeamId| framer_core::port::ChainConfig {
        max_total_tokens: Some(max_tokens),
        // Metered rungs are credit spend: off unless the operator opts in.
        metered_fallback: std::env::var("FRAMERIZE_ALLOW_METERED").map(|v| v == "1").unwrap_or(false),
        on_429: {
            let team = Some(team.0.clone());
            let rotation = s.serving_accounts.rotation().clone();
            Some(std::sync::Arc::new(move || {
                if let Some(t) = &team {
                    rotation.mark_saturated(t, 45_000);
                }
            }))
        },
        ..Default::default()
    };
    let mut cfg = mk_cfg(&scope_team);
    let mut account = account;
    if wants_stream {
        let body_residency = residency.clone();
        // INCREMENTAL streaming: spawn the chain with an events callback and
        // emit SSE frames as chunks land — message_start/role/created
        // immediately, per-chunk deltas as they arrive, heartbeat pings every
        // 8s of silence. The old behavior buffered the entire chain before
        // the first byte; streaming clients (Claude Code) timed out the
        // silent socket: "Claude gave up after repeated API errors".
        let msg_id = format!("msg_{:08x}{:08x}", rand_u32(), rand_u32());
        let (tx, rx) = tokio::sync::mpsc::channel::<convert::ChainEvent>(1024);
        let cancelled = tx.clone();
        cfg.cancelled = Some(Arc::new(move || cancelled.is_closed()));
        let disconnected = tx.clone();
        let s2 = s.clone();
        let account2 = account.clone();
        let project2 = project.clone();
        let mut chat_session2 = chat_session.clone();
        let explicit_chat_session = hdr(headers, "x-framerize-session");
        let log_model = anthro_model_for_log(dialect, &payload);
        // The task owns the first permit and reacquires for each replacement
        // account using that attempt's actual workspace and model.
        let (handle, completion) = operation_ownership::spawn_observed(s.operation_ownership.as_ref(),
            framer_core::operation_gate::OperationKind::Inference, async move {
            let t0 = std::time::Instant::now();
            // Tracks whether any CONTENT ever reached the stream: a chain that
            // dies before its first chunk can be silently restarted on a fresh
            // account (the client's message_start and pings are emitted by US,
            // so a swap mid-silence is invisible); once content has flowed, a
            // restart would duplicate blocks and the error must surface.
            let emitted_content = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let ec2 = emitted_content.clone();
            let cb = move |ev: convert::ChainEvent| {
                if matches!(ev, convert::ChainEvent::Chunk(_) | convert::ChainEvent::ResponsesItems { .. }) {
                    ec2.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) = tx.try_send(ev) {
                    // Channel full under client backpressure: log it. A dropped
                    // frame here is a hole in the user's stream — invisible
                    // before (2026-10-06 "stream stopped" audit).
                    eprintln!("[stream] event channel full — dropping a frame (client backpressure)");
                }
            };
            let mut account = account2;
            let mut session = session;
            let mut project = project2;
            let mut cfg2 = cfg;
            let mut tried: Vec<String> = Vec::new();
            let mut acct_attempt = 0u32;
            let mut cleanup_confirmed = true;
            let mut cooldown_wait = account_wait::MAX_WAIT;
            let result = loop {
                if let Err(error) = cfg2.check_cancelled() { break Err(error); }
                let permit = match first_permit.take() {
                    Some(permit) => permit,
                    None => tokio::select! {
                        _ = disconnected.closed() => break Err("client cancelled".into()),
                        permit = s2.gate.acquire(&gate_key) => permit,
                    },
                };
                if let Err(error) = cfg2.check_cancelled() { break Err(error); }
                let admission = current_account(&s2, account.id).await;
                let admitted = admission.is_ok();
                let r = match admission {
                    Err(error) => Err(chain_admission_error(error)),
                    Ok(_) => s2
                    .client
                    .freechat_long(
                        &session,
                        s2.browser.as_ref(),
                        &account.id,
                        &project,
                        anthro.clone(),
                        &cfg2,
                        &chat_session2,
                        Some(&cb),
                    )
                    .await,
                };
                drop(permit);
                cleanup_confirmed &= !matches!(&r, Err(error) if pool_control::cleanup_unconfirmed(error));
                let r = residency.observe_result(account.id, r);
                if admitted && !matches!(&r, Err(error) if error == "client cancelled") {
                    record_account_health(&s2, &account, &r).await;
                }
                if cfg2.is_cancelled() { break r; }
                let refusal = matches!(&r, Err(e) if should_retry_account(e));
                if allow_account_repick && refusal && !emitted_content.load(std::sync::atomic::Ordering::Relaxed) && acct_attempt < 2 {
                    let local = matches!(&r, Err(error) if AccountCooldown::from_error(error).is_some()
                        || is_server_cooling(error));
                    if !local { tried.push(account.id.0.to_string()); }
                    drop(residency.take());
                    let acquired = tokio::select! {
                        _ = disconnected.closed() => break Err("client cancelled".into()),
                        value = account_wait::pick(&s2, &tried,
                            account_wait::WaitFor::after_attempt(local, cleanup_confirmed),
                            &mut cooldown_wait) => value,
                    };
                    cleanup_confirmed &= acquired.cleanup_confirmed;
                    match acquired.result {
                        Ok((next, lease)) => {
                            residency.replace(lease);
                            {
                                use framer_core::observability::Event;
                                Event::new("ai.chain.repick")
                                    .account(&account.id)
                                    .field("to", &next.id.0.to_string()[..8])
                                    .field("reason", r.as_ref().err().map(|e| error_excerpt(e, 120)).unwrap_or(""))
                                    .emit();
                            }
                            account = next;
                            session = match s2.sessions.load(&account.framer_user_id).await {
                                Ok(Some(x)) => x,
                                _ => break r,
                            };
                            let sc = match pick_scope(&s2, &account) { Ok(sc) => sc, Err(_) => break r };
                            project = sc.project.clone();
                            gate_key = format!("{}:{gate_model}", sc.team.0);
                            chat_session2 = explicit_chat_session.clone()
                                .unwrap_or_else(|| stable_session(&account.id));
                            let team = Some(sc.team.0.clone());
                            let rotation = s2.serving_accounts.rotation().clone();
                            cfg2.on_429 = Some(std::sync::Arc::new(move || {
                                if let Some(t) = &team {
                                    rotation.mark_saturated(t, 45_000);
                                }
                            }));
                            acct_attempt += 1;
                            continue;
                        }
                        Err(_) => break r,
                    }
                }
                break r;
            };
            {
                use framer_core::observability::Event;
                let mut ev = operation_ownership::correlate_chain(Event::new("ai.chain"))
                    .account(&account.id)
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
            (result, cleanup_confirmed)
        }).map_err(ApiError::operation_admission)?;

        let open_frames = convert::stream_open(dialect, &client_model, &msg_id);
        let composer = convert::SseComposer::new_with_custom(dialect, &msg_id, &client_model, custom_tools.clone());
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
                        // ResponsesItems belong to the native path's own stream
                        // (native_complete); they never arrive on this channel.
                        Some(convert::ChainEvent::ResponsesItems { .. }) => vec![convert::ping_frame(dialect)],
                        // Channel closed (chain finishing) or Done: emit a ping
                        // instead of an EMPTY frame — a zero-length chunk is
                        // end-of-stream to strict SSE clients.
                        Some(convert::ChainEvent::Done(_)) | None => vec![convert::ping_frame(dialect)],
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
        let stream = retain_owner(stream, body_residency);
        let mut resp = axum::response::Response::new(axum::body::Body::from_stream(stream));
        resp.extensions_mut().insert(completion);
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
    let cancelled = operation_ownership::request_cancelled();
    cfg.cancelled = cancelled.clone();
    let mut result;
    let mut cleanup_confirmed = true;
    let mut tried_accounts: Vec<String> = Vec::new();
    let mut acct_attempt = 0u32;
    'account_retry: loop {
    {
        if let Err(error) = cfg.check_cancelled() {
            result = Err(error);
            break 'account_retry;
        }
        // Re-acquire per attempt so a retry waits for the narrowed limit.
        let permit = match first_permit.take() {
            Some(permit) => permit,
            None => tokio::select! {
                biased;
                _ = operation_ownership::request_disconnected() => {
                    result = Err("client cancelled".into());
                    break 'account_retry;
                }
                permit = s.gate.acquire(&gate_key) => permit,
            },
        };
        if let Err(error) = cfg.check_cancelled() {
            result = Err(error);
            break 'account_retry;
        }
        if let Err(error) = current_account(&s, account.id).await {
            return Ok(operation_ownership::completed_response(
                axum::response::IntoResponse::into_response(error), cleanup_confirmed));
        }
        if let Err(error) = cfg.check_cancelled() {
            result = Err(error);
            break 'account_retry;
        }
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
        // Later success cannot confirm cleanup lost by an earlier attempt.
        cleanup_confirmed &= !matches!(&result, Err(error) if pool_control::cleanup_unconfirmed(error));
        result = residency.observe_result(account.id, result);
        // Retain the completed call's health outcome even after disconnect.
        record_account_health(&s, &account, &result).await;
        // The chain owns the bounded rate retry budget. Do not start it again
        // after exhaustion or replay chunks already delivered by that chain.
    }

    // Local capacity recovery retains the existing ownership checks.
    // Security refusals are final for this request.
    if cfg.is_cancelled() { break; }
    if allow_account_repick && matches!(&result, Err(e) if should_retry_account(e)) && acct_attempt < 2 {
        tried_accounts.push(account.id.0.to_string());
        drop(residency.take());
        let acquired = pick_fresh_account(&s, &tried_accounts).await;
        cleanup_confirmed &= acquired.cleanup_confirmed;
        match acquired.result {
            Ok((next, lease)) => {
                residency.replace(lease);
                if cfg.is_cancelled() { break; }
                {
                    use framer_core::observability::Event;
                    Event::new("ai.chain.repick")
                        .account(&account.id)
                        .field("to", &next.id.0.to_string()[..8])
                        .field("reason", result.as_ref().err().map(|e| error_excerpt(e, 120)).unwrap_or(""))
                        .emit();
                }
                account = next;
                session = match s.sessions.load(&account.framer_user_id).await {
                    Ok(Some(s2)) => s2,
                    _ => break 'account_retry,
                };
                scope = match pick_scope(&s, &account) { Ok(sc) => sc, Err(_) => break 'account_retry };
                project = scope.project.clone();
                scope_team = scope.team.clone();
                gate_key = format!("{}:{gate_model}", scope_team.0);
                cfg = mk_cfg(&scope_team);
                cfg.cancelled = cancelled.clone();
                chat_session = hdr(headers, "x-framerize-session")
                    .unwrap_or_else(|| stable_session(&account.id));
                acct_attempt += 1;
                continue 'account_retry;
            }
            Err(_) => break 'account_retry,
        }
    }
    break;
    } // end account_retry

    // Feed the outcome back to the gate so the limit tracks reality: 429 means
    // "slow down", never "this account is bad" (AGENTS.md rule 3).
    match &result {
        Ok(_) => s.gate.on_success(&gate_key).await,
        Err(e) if framer_core::error::is_workspace_saturation(e) => {
            s.gate.on_overloaded(&gate_key).await;
            if let Some(team) = account.current_team.as_ref() {
                s.serving_accounts.rotation().mark_saturated(&team.0, 45_000);
            }
        }
        Err(_) => {}
    }

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
    let result = match result {
        Ok(r) => r,
        Err(e) => {
            let (status, body) = convert::error_json(dialect, &e);
            let response = axum::response::IntoResponse::into_response(ShapedApiError(
                StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                body,
            ));
            return Ok(operation_ownership::completed_response(response,
                cleanup_confirmed && !pool_control::cleanup_unconfirmed(&e)));
        }
    };

    let body = convert::render_json_with(dialect, &result, &client_model, &custom_tools);
    let body = match serde_json::to_string(&body) {
        Ok(body) => body,
        Err(error) => return Ok(operation_ownership::completed_response(
            axum::response::IntoResponse::into_response(ApiError::Internal(error.to_string())),
            cleanup_confirmed)),
    };
    let mut resp = axum::response::Response::new(axum::body::Body::from(
        body,
    ));
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    resp.headers_mut().insert(
        "x-framerize-session",
        chat_session.parse().unwrap_or(axum::http::HeaderValue::from_static("")),
    );
    Ok(operation_ownership::completed_response(resp, cleanup_confirmed))
}

/// The OpenAI-native free path (docs/native-chains.md): Responses clients
/// run the responses-native chain verbatim; chat clients map thinly onto it.
/// No Anthropic conversion for OpenAI clients — tools, reasoning items,
/// service tiers, and ids all survive by construction.
fn prepare_native_payload(dialect: ClientDialect, payload: Value) -> Result<Value, String> {
    let mut native_payload = if dialect == ClientDialect::OpenAiChat {
        convert::chat_to_responses_request(&payload)?
    } else {
        payload
    };
    // The split-writes rule (same as the anthro chain): with tools present,
    // tell the model to chunk big writes — the strongest mid-call-cut
    // prevention there is.
    let has_tools = native_payload["tools"].as_array().map(|a| !a.is_empty()).unwrap_or(false);
    if has_tools {
        let note = "When using tools to write or edit files, split large writes: at most ~40 lines per tool call, then append the next section with a separate call, and repeat until done.";
        match native_payload["instructions"].as_str() {
            Some(existing) if !existing.is_empty() => {
                native_payload["instructions"] = serde_json::Value::String(format!("{existing}\n\n{note}"));
            }
            _ => {
                native_payload["instructions"] = serde_json::Value::String(note.into());
            }
        }
    }

    Ok(native_payload)
}

async fn native_complete(
    s: AppState,
    headers: &axum::http::HeaderMap,
    dialect: ClientDialect,
    payload: serde_json::Value,
    wants_stream: bool,
    include_usage: bool,
    max_tokens: Option<u32>,
) -> Result<axum::response::Response, ApiError> {
    let client_model = payload["model"].as_str().unwrap_or_default().to_string();
    let mut native_payload = if dialect == ClientDialect::OpenAiChat {
        convert::chat_to_responses_request(&payload)
            .map_err(ApiError::Internal).map_err(operation_ownership::completed_preflight)?
    } else {
        payload.clone()
    };
    // The split-writes rule (same as the anthro chain): with tools present,
    // tell the model to chunk big writes — the strongest mid-call-cut
    // prevention there is.
    let has_tools = native_payload["tools"].as_array().map(|a| !a.is_empty()).unwrap_or(false);
    if has_tools {
        let note = "When using tools to write or edit files, split large writes: at most ~40 lines per tool call, then append the next section with a separate call, and repeat until done.";
        match native_payload["instructions"].as_str() {
            Some(existing) if !existing.is_empty() => {
                native_payload["instructions"] = serde_json::Value::String(format!("{existing}\n\n{note}"));
            }
            _ => {
                native_payload["instructions"] = serde_json::Value::String(note.into());
            }
        }
    }

    if let Some(remaining) = s.wave.check() {
        if wants_stream && dialect == ClientDialect::OpenAiResponses
            && !headers.contains_key("x-framerize-account") {
            return response_output::wait(s, native_payload, client_model, max_tokens,
                hdr(headers, "x-framerize-session")).await;
        }
        return Err(operation_ownership::completed_preflight(ApiError::Cooling {
            retry_after_secs: remaining.as_secs().saturating_add(1),
        }));
    }
    if headers.get("x-framerize-account").is_some_and(|value| value.to_str().is_err()) {
        return Err(operation_ownership::completed_preflight(ApiError::InvalidRequest(
            "x-framerize-account must contain valid text".into(),
        )));
    }
    let requested_account = hdr(headers, "x-framerize-account");
    let allow_account_repick = requested_account.is_none();
    let gate_model = convert::to_openrouter_id(&client_model);
    // Recheck queued work before opening SSE, without reusing a cooling account.
    let mut skipped = Vec::new();
    let (account, residency, mut session, mut scope, mut first_permit) = loop {
        let pool = s.state.list().await.map_err(ApiError::Internal)?;
        let acquired = pool_control::select_account_observed(
            &s, pool, requested_account.as_deref(), &skipped).await;
        let lease = match acquired.result {
            Ok(lease) => lease,
            Err(ApiError::Cooling { .. } | ApiError::AccountCooling { .. })
                if acquired.cleanup_confirmed && allow_account_repick && wants_stream
                && dialect == ClientDialect::OpenAiResponses => {
                return response_output::wait(s, native_payload, client_model, max_tokens,
                    hdr(headers, "x-framerize-session")).await;
            }
            Err(error) if skipped.is_empty() && acquired.cleanup_confirmed =>
                return Err(operation_ownership::completed_preflight(error)),
            Err(error) => return Ok(operation_ownership::completed_response(
                axum::response::IntoResponse::into_response(error), acquired.cleanup_confirmed)),
        };
        let selected_id = lease.account();
        let residency = RequestResidency::new(lease);
        let account = match current_account(&s, selected_id).await {
            Ok(account) => account,
            Err(ApiError::AccountCooling { .. }) if allow_account_repick && skipped.len() < 2 => {
                skipped.push(selected_id.0.to_string());
                continue;
            },
            Err(ApiError::Cooling { .. } | ApiError::AccountCooling { .. })
                if allow_account_repick && wants_stream && dialect == ClientDialect::OpenAiResponses => {
                drop(residency);
                return response_output::wait(s, native_payload, client_model, max_tokens,
                    hdr(headers, "x-framerize-session")).await;
            },
            Err(error @ (ApiError::Cooling { .. } | ApiError::AccountCooling { .. }
                | ApiError::ServingCapacity(_))) => {
                return Ok(operation_ownership::completed_response(
                    axum::response::IntoResponse::into_response(error), true));
            },
            Err(error) => return Err(error),
        };
        let session = s
            .sessions
            .load(&account.framer_user_id)
            .await
            .map_err(ApiError::Internal)?
            .ok_or_else(|| ApiError::Internal("account has no session".into()))?;
        let scope = pick_scope(&s, &account)?;

        // Per (workspace × MODEL) concurrency, keyed on the normalized upstream id
        // and the chain's rotated workspace (multi-workspace goose).
        let gate_key = format!("{}:{gate_model}", scope.team.0);
        let first_permit = Some(tokio::select! {
            biased;
            _ = operation_ownership::request_disconnected() => {
                let (status, body) = convert::error_json(dialect, "client cancelled");
                let response = axum::response::IntoResponse::into_response(ShapedApiError(
                    StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), body,
                ));
                return Ok(operation_ownership::completed_response(response, true));
            }
            permit = s.gate.acquire(&gate_key) => permit,
        });
        match current_account(&s, account.id).await {
            Ok(_) => {},
            Err(ApiError::AccountCooling { .. }) if allow_account_repick && skipped.len() < 2 => {
                skipped.push(account.id.0.to_string());
                continue;
            },
            Err(ApiError::Cooling { .. } | ApiError::AccountCooling { .. })
                if allow_account_repick && wants_stream && dialect == ClientDialect::OpenAiResponses => {
                drop(first_permit);
                drop(residency);
                return response_output::wait(s, native_payload, client_model, max_tokens,
                    hdr(headers, "x-framerize-session")).await;
            },
            Err(error @ (ApiError::Cooling { .. } | ApiError::AccountCooling { .. }
                | ApiError::ServingCapacity(_))) => {
                return Ok(operation_ownership::completed_response(
                    axum::response::IntoResponse::into_response(error), true));
            },
            Err(error) => return Err(error),
        }

        break (account, residency, session, scope, first_permit);
    };
    let mut project = scope.project.clone();
    let mut scope_team = scope.team.clone();
    let mut chat_session = hdr(headers, "x-framerize-session")
        .unwrap_or_else(|| stable_session(&account.id));
    let mut gate_key = format!("{}:{gate_model}", scope_team.0);
    let cancelled = operation_ownership::request_cancelled();
    let mk_cfg = |team: &framer_core::ids::TeamId| framer_core::port::ChainConfig {
        max_total_tokens: max_tokens,
        cancelled: cancelled.clone(),
        metered_fallback: std::env::var("FRAMERIZE_ALLOW_METERED").map(|v| v == "1").unwrap_or(false),
        on_429: {
            let team = Some(team.0.clone());
            let rotation = s.serving_accounts.rotation().clone();
            Some(std::sync::Arc::new(move || {
                if let Some(t) = &team {
                    rotation.mark_saturated(t, 45_000);
                }
            }))
        },
        ..Default::default()
    };
    let mut cfg = mk_cfg(&scope_team);
    let mut account = account;

    // ONE logical id for the whole response (minted here, before the composer
    // sends response.created): chunk ids never surface to the client.
    let logical_id = format!("{:08x}{:08x}", rand_u32(), rand_u32());

    if wants_stream && dialect == ClientDialect::OpenAiResponses {
        return response_output::serve(response_output::FreeRequest {
            state: s, account, residency, session, project, config: cfg,
            payload: native_payload, gate_model, gate_key, first_permit,
            chat_session, explicit_session: hdr(headers, "x-framerize-session"),
            allow_repick: allow_account_repick, client_model, logical_id,
            wait_budget: account_wait::MAX_WAIT,
        }).await;
    }

    if wants_stream {
        let body_residency = residency.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<convert::ChainEvent>(1024);
        let cancelled = tx.clone();
        cfg.cancelled = Some(Arc::new(move || cancelled.is_closed()));
        let disconnected = tx.clone();
        let s2 = s.clone();
        let account2 = account.clone();
        let project2 = project.clone();
        let mut chat_session2 = chat_session.clone();
        let explicit_chat_session = hdr(headers, "x-framerize-session");
        let log_model = gate_model.clone();
        let (handle, completion) = operation_ownership::spawn_observed(s.operation_ownership.as_ref(),
            framer_core::operation_gate::OperationKind::Inference, async move {
            let t0 = std::time::Instant::now();
            let emitted_content = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let ec2 = emitted_content.clone();
            let cb = move |ev: convert::ChainEvent| {
                if matches!(ev, convert::ChainEvent::Chunk(_) | convert::ChainEvent::ResponsesItems { .. }) {
                    ec2.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) = tx.try_send(ev) {
                    // Channel full under client backpressure: log it. A dropped
                    // frame here is a hole in the user's stream — invisible
                    // before (2026-10-06 "stream stopped" audit).
                    eprintln!("[stream] event channel full — dropping a frame (client backpressure)");
                }
            };
            let mut account = account2;
            let mut session = session;
            let mut project = project2;
            let mut cfg2 = cfg;
            let mut tried: Vec<String> = Vec::new();
            let mut acct_attempt = 0u32;
            let mut cleanup_confirmed = true;
            let mut cooldown_wait = account_wait::MAX_WAIT;
            let result = loop {
                if let Err(error) = cfg2.check_cancelled() { break Err(error); }
                let permit = match first_permit.take() {
                    Some(permit) => permit,
                    None => tokio::select! {
                        _ = disconnected.closed() => break Err("client cancelled".into()),
                        permit = s2.gate.acquire(&gate_key) => permit,
                    },
                };
                if let Err(error) = cfg2.check_cancelled() { break Err(error); }
                let admission = current_account(&s2, account.id).await;
                let admitted = admission.is_ok();
                let r = match admission {
                    Err(error) => Err(chain_admission_error(error)),
                    Ok(_) => s2
                    .client
                    .native_long(
                        &session,
                        s2.browser.as_ref(),
                        &account.id,
                        &project,
                        native_payload.clone(),
                        &cfg2,
                        &chat_session2,
                        Some(&cb),
                    )
                    .await,
                };
                drop(permit);
                cleanup_confirmed &= !matches!(&r, Err(error) if pool_control::cleanup_unconfirmed(error));
                let r = residency.observe_result(account.id, r);
                if admitted && !matches!(&r, Err(error) if error == "client cancelled") {
                    record_account_health(&s2, &account, &r).await;
                }
                if cfg2.is_cancelled() { break r; }
                let refusal = matches!(&r, Err(e) if should_retry_account(e));
                if allow_account_repick && refusal && !emitted_content.load(std::sync::atomic::Ordering::Relaxed) && acct_attempt < 2 {
                    let local = matches!(&r, Err(error) if AccountCooldown::from_error(error).is_some()
                        || is_server_cooling(error));
                    if !local { tried.push(account.id.0.to_string()); }
                    drop(residency.take());
                    let acquired = tokio::select! {
                        _ = disconnected.closed() => break Err("client cancelled".into()),
                        value = account_wait::pick(&s2, &tried,
                            account_wait::WaitFor::after_attempt(local, cleanup_confirmed),
                            &mut cooldown_wait) => value,
                    };
                    cleanup_confirmed &= acquired.cleanup_confirmed;
                    match acquired.result {
                        Ok((next, lease)) => {
                            residency.replace(lease);
                            {
                                use framer_core::observability::Event;
                                Event::new("ai.chain.repick")
                                    .account(&account.id)
                                    .field("to", &next.id.0.to_string()[..8])
                                    .field("reason", r.as_ref().err().map(|e| error_excerpt(e, 120)).unwrap_or(""))
                                    .emit();
                            }
                            account = next;
                            session = match s2.sessions.load(&account.framer_user_id).await {
                                Ok(Some(x)) => x,
                                _ => break r,
                            };
                            let sc = match pick_scope(&s2, &account) { Ok(sc) => sc, Err(_) => break r };
                            project = sc.project.clone();
                            gate_key = format!("{}:{gate_model}", sc.team.0);
                            chat_session2 = explicit_chat_session.clone()
                                .unwrap_or_else(|| stable_session(&account.id));
                            let team = Some(sc.team.0.clone());
                            let rotation = s2.serving_accounts.rotation().clone();
                            cfg2.on_429 = Some(std::sync::Arc::new(move || {
                                if let Some(t) = &team {
                                    rotation.mark_saturated(t, 45_000);
                                }
                            }));
                            acct_attempt += 1;
                            continue;
                        }
                        Err(_) => break r,
                    }
                }
                break r;
            };
            let mut ev = operation_ownership::correlate_chain(
                framer_core::observability::Event::new("ai.chain"))
                .account(&account.id)
                .field("model", log_model)
                .field("dialect", format!("{dialect:?}"))
                .duration_ms(t0.elapsed().as_millis());
            ev = match &result {
                Ok(r) => ev
                    .field("chunks", r.chunks)
                    .field("metered_chunks", r.metered_chunks)
                    .field("prompt_tokens", r.input_tokens)
                    .field("completion_tokens", r.output_tokens)
                    .field("finish_reason", r.status.clone())
                    .outcome("ok"),
                Err(e) => ev.error(e),
            };
            ev.emit();
            (result, cleanup_confirmed)
        }).map_err(ApiError::operation_admission)?;

        let mut composer = NativeComposerState::new(dialect, &client_model, &logical_id);
        let open_frames = composer.open(&client_model, &logical_id);
        let stream = futures::stream::unfold(
            (rx, open_frames, Some(composer), false, handle, include_usage, dialect),
            |(mut rx, mut pending, mut composer, mut done, mut handle, include_usage, dialect)| async move {
                if done {
                    return None;
                }
                if !pending.is_empty() {
                    let out = pending.join("\n\n") + "\n\n";
                    pending.clear();
                    return Some((Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(out)), (rx, pending, composer, done, handle, include_usage, dialect)));
                }
                let frames: Vec<String> = tokio::select! {
                    biased;
                    r = &mut handle => {
                        done = true;
                        let mut drained: Vec<String> = Vec::new();
                        while let Ok(ev) = rx.try_recv() {
                            if let convert::ChainEvent::ResponsesItems { items, continues_text } = ev {
                                drained.extend(composer.as_mut().unwrap().push(&items, continues_text));
                            }
                        }
                        let tail = match r {
                            Ok(Ok(assembled)) => composer.take().unwrap().finish(&assembled, include_usage),
                            Ok(Err(e)) => composer.take().unwrap().fail(&e),
                            Err(error) if dialect == ClientDialect::OpenAiResponses =>
                                composer.take().unwrap().fail(&if error.is_panic() {
                                    framer_core::port::OutputError::ProducerPanicked
                                } else { framer_core::port::OutputError::ProducerCancelled }.to_string()),
                            Err(je) => vec![convert::ping_frame(dialect), format!(": join error {je}")],
                        };
                        drained.into_iter().chain(tail).collect()
                    }
                    // A closed channel must not spin while the producer joins cleanup.
                    ev = rx.recv(), if !(rx.is_closed() && rx.is_empty()) => match ev {
                        Some(convert::ChainEvent::ResponsesItems { items, continues_text }) => {
                            composer.as_mut().unwrap().push(&items, continues_text)
                        }
                        Some(_) => vec![convert::ping_frame(dialect)],
                        None => vec![convert::ping_frame(dialect)],
                    },
                    _ = tokio::time::sleep(std::time::Duration::from_secs(8)) => {
                        vec![composer.as_mut().unwrap().heartbeat()]
                    }
                };
                let out = if frames.is_empty() { String::new() } else { frames.join("\n\n") + "\n\n" };
                Some((Ok(axum::body::Bytes::from(out)), (rx, pending, composer, done, handle, include_usage, dialect)))
            },
        );
        let stream = retain_owner(stream, body_residency);
        let mut resp = axum::response::Response::new(axum::body::Body::from_stream(stream));
        resp.extensions_mut().insert(completion);
        resp.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
        resp.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        );
        resp.headers_mut().insert(
            "x-framerize-session",
            chat_session.parse().unwrap_or(axum::http::HeaderValue::from_static("")),
        );
        return Ok(resp);
    }

    let t0 = std::time::Instant::now();
    let mut tried_accounts: Vec<String> = Vec::new();
    let mut acct_attempt = 0u32;
    let mut cleanup_confirmed = true;
    let result = 'account_retry: loop {
        if let Err(error) = cfg.check_cancelled() { break Err(error); }
        let permit = match first_permit.take() {
            Some(permit) => permit,
            None => tokio::select! {
                biased;
                _ = operation_ownership::request_disconnected() => {
                    break 'account_retry Err("client cancelled".into());
                }
                permit = s.gate.acquire(&gate_key) => permit,
            },
        };
        if let Err(error) = cfg.check_cancelled() { break Err(error); }
        if let Err(error) = current_account(&s, account.id).await {
            return Ok(operation_ownership::completed_response(
                axum::response::IntoResponse::into_response(error), cleanup_confirmed));
        }
        let r = s
            .client
            .native_long(
                &session,
                s.browser.as_ref(),
                &account.id,
                &project,
                native_payload.clone(),
                &cfg,
                &chat_session,
                None,
            )
            .await;
        drop(permit);
        let r = residency.observe_result(account.id, r);
        // A later successful account cannot confirm an earlier lost cleanup.
        cleanup_confirmed &= !matches!(&r, Err(error) if pool_control::cleanup_unconfirmed(error));
        record_account_health(&s, &account, &r).await;
        // Nothing reaches the client on the non-stream path until the chain
        // completes, so an unbound request may retry a Castle refusal on a
        // fresh account (the verdict follows the account, not the request).
        if cfg.is_cancelled() { break r; }
        if allow_account_repick && matches!(&r, Err(e) if should_retry_account(e)) && acct_attempt < 2 {
            tried_accounts.push(account.id.0.to_string());
            drop(residency.take());
            let acquired = pick_fresh_account(&s, &tried_accounts).await;
            cleanup_confirmed &= acquired.cleanup_confirmed;
            match acquired.result {
                Ok((next, lease)) => {
                    residency.replace(lease);
                    {
                        use framer_core::observability::Event;
                        Event::new("ai.chain.repick")
                            .account(&account.id)
                            .field("to", &next.id.0.to_string()[..8])
                            .field("reason", r.as_ref().err().map(|e| error_excerpt(e, 120)).unwrap_or(""))
                            .emit();
                    }
                    account = next;
                    session = match s.sessions.load(&account.framer_user_id).await {
                        Ok(Some(s2)) => s2,
                        _ => break 'account_retry r,
                    };
                    scope = match pick_scope(&s, &account) { Ok(sc) => sc, Err(_) => break 'account_retry r };
                    project = scope.project.clone();
                    scope_team = scope.team.clone();
                    gate_key = format!("{}:{gate_model}", scope_team.0);
                    cfg = mk_cfg(&scope_team);
                    chat_session = hdr(headers, "x-framerize-session")
                        .unwrap_or_else(|| stable_session(&account.id));
                    acct_attempt += 1;
                    continue 'account_retry;
                }
                Err(_) => break 'account_retry r,
            }
        }
        break 'account_retry r;
    };
    {
        let mut ev = framer_core::observability::Event::new("ai.chain")
            .account(&account.id)
            .field("model", gate_model.clone())
            .field("dialect", format!("{dialect:?}"))
            .duration_ms(t0.elapsed().as_millis());
        ev = match &result {
            Ok(r) => ev
                .field("chunks", r.chunks)
                .field("metered_chunks", r.metered_chunks)
                .field("prompt_tokens", r.input_tokens)
                .field("completion_tokens", r.output_tokens)
                .field("finish_reason", r.status.clone())
                .outcome("ok"),
            Err(e) => ev.error(e),
        };
        ev.emit();
    }
    let result = match result {
        Ok(r) => r,
        Err(e) => {
            let (status, body) = convert::error_json(dialect, &e);
            let response = axum::response::IntoResponse::into_response(ShapedApiError(
                StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                body,
            ));
            return Ok(operation_ownership::completed_response(response,
                cleanup_confirmed && !pool_control::cleanup_unconfirmed(&e)));
        }
    };
    let body = if dialect == ClientDialect::OpenAiChat {
        convert::responses_to_chat_completion(&result, &client_model)
    } else {
        convert::responses_json(&result, &client_model, &format!("resp_{logical_id}"))
    };
    let body = match serde_json::to_string(&body) {
        Ok(body) => body,
        Err(error) => return Ok(operation_ownership::completed_response(
            axum::response::IntoResponse::into_response(ApiError::Internal(error.to_string())),
            cleanup_confirmed)),
    };
    let mut resp = axum::response::Response::new(axum::body::Body::from(
        body,
    ));
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    resp.headers_mut().insert(
        "x-framerize-session",
        chat_session.parse().unwrap_or(axum::http::HeaderValue::from_static("")),
    );
    Ok(operation_ownership::completed_response(resp, cleanup_confirmed))
}

/// Per-dialect composer for the native stream (responses SSE or chat chunks).
enum NativeComposerState {
    Responses(convert::ResponsesComposer),
    Chat(convert::ChatComposer),
}

impl NativeComposerState {
    fn new(dialect: ClientDialect, client_model: &str, logical_id: &str) -> Self {
        match dialect {
            ClientDialect::OpenAiChat => {
                Self::Chat(convert::ChatComposer::new(client_model, logical_id))
            }
            _ => Self::Responses(convert::ResponsesComposer::new(client_model, logical_id)),
        }
    }
    fn open(&mut self, client_model: &str, logical_id: &str) -> Vec<String> {
        match self {
            Self::Responses(c) => c.open(),
            Self::Chat(_) => convert::stream_open(ClientDialect::OpenAiChat, client_model, logical_id),
        }
    }
    fn heartbeat(&mut self) -> String {
        match self {
            Self::Responses(c) => c.heartbeat(),
            Self::Chat(_) => convert::ping_frame(ClientDialect::OpenAiChat),
        }
    }
    fn push(&mut self, items: &[serde_json::Value], continues_text: bool) -> Vec<String> {
        match self {
            Self::Responses(c) => c.push_items(items, continues_text),
            Self::Chat(c) => c.push_items(items, continues_text),
        }
    }
    fn finish(self, r: &convert::AssembledResponses, include_usage: bool) -> Vec<String> {
        match self {
            Self::Responses(c) => c.finish(r),
            Self::Chat(c) => c.finish(r, include_usage),
        }
    }
    fn fail(self, message: &str) -> Vec<String> {
        match self {
            Self::Responses(c) => c.fail(message),
            Self::Chat(_) => convert::error_frames(ClientDialect::OpenAiChat, message),
        }
    }
}

fn anthro_model_for_log(dialect: ClientDialect, payload: &Value) -> String {
    let _ = dialect;
    convert::to_openrouter_id(payload["model"].as_str().unwrap_or_default())
}

/// Anthropic count_tokens shape; character-based estimate (≈4 chars/token) —
/// the same fallback Claude Code applies when the endpoint is absent.
async fn count_tokens(body: Result<Json<Value>, JsonRejection>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Json(body) = match body {
        Ok(body) => body,
        Err(error) => return operation_ownership::completed_response(error.into_response(), true),
    };
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
    operation_ownership::completed_response(
        Json(serde_json::json!({ "input_tokens": (chars / 4).max(1) })).into_response(), true)
}

/// Paid-path wrappers: the metered streaming passthroughs, mounted at /paid/*.
async fn anthropic_messages_paid(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    payload: Result<Json<Value>, JsonRejection>,
) -> Result<axum::response::Response, ApiError> {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(error) => return Ok(operation_ownership::completed_response(
            axum::response::IntoResponse::into_response(error), true)),
    };
    compat_forward_headers(
        s,
        "anthropic-messages",
        &headers,
        payload,
    )
    .await
}

async fn openai_responses_paid(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    payload: Result<Json<Value>, JsonRejection>,
) -> Result<axum::response::Response, ApiError> {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(error) => return Ok(operation_ownership::completed_response(
            axum::response::IntoResponse::into_response(error), true)),
    };
    compat_forward_headers(
        s,
        "openai-responses",
        &headers,
        payload,
    )
    .await
}

async fn openai_chat_paid(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    payload: Result<Json<Value>, JsonRejection>,
) -> Result<axum::response::Response, ApiError> {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(error) => return Ok(operation_ownership::completed_response(
            axum::response::IntoResponse::into_response(error), true)),
    };
    compat_forward_headers(
        s,
        "openrouter-chat",
        &headers,
        payload,
    )
    .await
}

fn hdr(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
}

/// Team ids already held by OTHER accounts. `exclude` is the account being
/// admitted, so a re-login keeps the workspace it already owns.

/// Populate account.extra_workspaces: every OTHER credited workspace the
/// account owns, each with a project (adopt newest, else create). This is the
/// multi-workspace concurrency goose — chains round-robin these scopes so one
/// account uses ALL its per-workspace 429 budgets (measured: the cap is per
/// workspace; devices do NOT multiply it).
async fn populate_extra_scopes(
    s: &AppState,
    session: &framer_core::port::Session,
    account: &mut ManagedAccount,
    claimed: &std::collections::BTreeSet<String>,
    cleanup: &pool_control::CleanupEvidence,
) {
    let deps = Deps {
        client: s.client.as_ref(),
        browser: s.browser.as_ref(),
        sessions: s.sessions.as_ref(),
        state: s.state.as_ref(),
        account_updates: &s.serving_accounts.health_update,
    };
    let credited = match cleanup.core(framer_core::credits::scan_workspaces(&deps, session).await) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[provision] extra-scope scan failed (non-fatal): {e}");
            return;
        }
    };
    let primary = account.current_team.as_ref().map(|t| t.0.clone());
    let mut scopes = Vec::new();
    for w in credited {
        if Some(w.team.0.clone()) == primary {
            continue; // the primary scope lives in current_team/current_project
        }
        if claimed.contains(&w.team.0) {
            continue;
        }
        if w.snapshot.available <= 0.0 {
            continue; // 0-credit workspaces 402 on every route — not a scope
        }
        let project_result = s
            .client
            .team_projects(session, &w.team)
            .await;
        let mut project = cleanup.external(project_result)
            .ok()
            .and_then(|ps| ps.into_iter().next());
        if project.is_none() {
            match cleanup.external(s.client.create_project(session, &w.team).await) {
                Ok(p) => {
                    eprintln!("[provision] created project {} in extra workspace {}", p.0, dedupe_identity_preview(&w.team.0));
                    project = Some(p);
                }
                Err(e) => {
                    eprintln!("[provision] project creation in extra workspace failed: {e}");
                    continue;
                }
            }
        }
        if let Some(project) = project {
            scopes.push(framer_core::account::WorkspaceScope { team: w.team.clone(), project });
        }
    }
    if !scopes.is_empty() {
        eprintln!("[provision] account now spans {} workspace scopes (multi-workspace concurrency)", scopes.len() + 1);
    }
    account.extra_workspaces = scopes;
}

async fn claimed_workspaces(
    s: &AppState,
    exclude: AccountId,
) -> Result<std::collections::BTreeSet<String>, ApiError> {
    let pool = s.state.list().await.map_err(ApiError::Internal)?;
    Ok(pool
        .iter()
        .filter(|a| a.id != exclude)
        .flat_map(|a| {
            a.current_team.iter().chain(a.extra_workspaces.iter().map(|scope| &scope.team))
        })
        .map(|team| team.0.clone())
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
async fn remove_account(state: State<AppState>, body: Json<RemoveBody>) -> Result<Json<serde_json::Value>, ApiError> {
    remove_account_observed(state, body).await.result
}

async fn remove_account_observed(State(s): State<AppState>, Json(body): Json<RemoveBody>)
    -> pool_control::CleanupOutcome<Json<serde_json::Value>> {
    let cleanup = pool_control::CleanupEvidence::new();
    let result = async {
    let _management = cleanup.outcome(pool_control::management_observed(&s, body.account_id, false).await)?;
    let existing = s.state.get(body.account_id).await.map_err(ApiError::Internal)?;
    let Some(acc) = existing else {
        return Err(ApiError::Internal("account not found".into()));
    };
    cleanup.outcome(s.serving_accounts.park_managed_observed(s.browser.as_ref(), body.account_id).await)?;
    if body.wipe_profile {
        // Best effort: a missing browser profile must not block the removal.
        let _ = cleanup.external(s.browser.rotate_identity(&body.account_id).await);
    }
    s.state.remove(body.account_id).await.map_err(ApiError::Internal)?;
    Ok(Json(serde_json::json!({
        "status": "removed",
        "account": acc.email.unwrap_or(acc.alias),
    })))
    }.await;
    cleanup.finish(result)
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
async fn dedupe_accounts(state: State<AppState>, body: Json<DedupeBody>) -> Result<Json<serde_json::Value>, ApiError> {
    dedupe_accounts_observed(state, body).await.result
}

fn dedupe_identity_preview(value: &str) -> &str {
    let mut end = value.len().min(8);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

async fn dedupe_accounts_observed(State(s): State<AppState>, Json(body): Json<DedupeBody>)
    -> pool_control::CleanupOutcome<Json<serde_json::Value>> {
    let cleanup = pool_control::CleanupEvidence::new();
    let result = async {
    let pool = s.state.list().await.map_err(ApiError::Internal)?;
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
            .map_err(ApiError::Internal)?
            .is_some();
        if a.current_project.is_none() && !has_credits && !has_session {
            seen_devices.remove(&a.castle_device_id);
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
            "team": a.current_team.as_ref().map(|t| dedupe_identity_preview(&t.0).to_string()),
            "device": dedupe_identity_preview(&a.castle_device_id).to_string(),
        }));
    }

    let mut removed = Vec::new();
    for (dup, why) in doomed {
        let name = dup.email.clone().unwrap_or_else(|| dup.alias.clone());
        if body.apply {
            let _management = cleanup.outcome(pool_control::management_observed(&s, dup.id, false).await)?;
            let current = s.state.get(dup.id).await.map_err(ApiError::Internal)?;
            let current = serde_json::to_value(current).map_err(|e| ApiError::Internal(e.to_string()))?;
            let expected = serde_json::to_value(Some(&dup)).map_err(|e| ApiError::Internal(e.to_string()))?;
            if current != expected {
                return Err(ApiError::PoolBusy("account changed during deduplication; retry the preview".into()));
            }
            cleanup.outcome(s.serving_accounts.park_managed_observed(s.browser.as_ref(), dup.id).await)?;
            s.state.remove(dup.id).await.map_err(ApiError::Internal)?;
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
    }.await;
    cleanup.finish(result)
}

#[derive(Deserialize)]
struct DedupeBody {
    /// Dry run unless true, so the operator sees the plan first.
    #[serde(default)]
    apply: bool,
}

async fn accounts(State(s): State<AppState>) -> Result<Json<Vec<ManagedAccount>>, ApiError> {
    Ok(Json(s.state.list().await.map_err(ApiError::Internal)?))
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
    let pool = s.state.list().await.map_err(ApiError::Internal)?;
    let now = framer_core::rotation::now_ms();
    let residency = s.serving_accounts.residents.snapshot()
        .map_err(|e| ApiError::PoolBusy(e.to_string()))?;
    let protected = s.serving_accounts.mutations.lock()
        .map_err(|_| ApiError::PoolBusy("account management is unavailable".into()))?.clone();

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
                        "account_id": a.id,
                        "name": a.email.clone().unwrap_or_else(|| a.alias.clone()),
                        "state": format!("{:?}", a.state),
                        "serving_allowed": s.serving_accounts.allows(&a.id),
                        "available": s.serving_accounts.allows(&a.id) && a.is_available_at(now)
                            && ServingAccountPolicy::residency_allows(&residency, &a.id, &protected),
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
        "available_now": pool.iter().filter(|a| s.serving_accounts.allows(&a.id)
            && a.is_available_at(now)
            && ServingAccountPolicy::residency_allows(&residency, &a.id, &protected)).count(),
        "serving_policy": {
            "mode": if s.serving_accounts.configured_count().is_some() { "runtime_allowlist" } else { "all_accounts" },
            "configured_accounts": s.serving_accounts.configured_count(),
            "allowed_records": pool.iter().filter(|a| s.serving_accounts.allows(&a.id)).count(),
            "parked_records": pool.iter().filter(|a| !s.serving_accounts.allows(&a.id)).count(),
            "browser_residency": "tracked_slots",
            "changes_require_restart": false,
        },
        "config": s.serving_accounts.config(),
        "residency": residency,
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
async fn begin_login(state: State<AppState>, body: Json<BeginLoginBody>) -> Result<&'static str, ApiError> {
    begin_login_observed(state, body).await.result
}

async fn begin_login_observed(State(s): State<AppState>, Json(body): Json<BeginLoginBody>)
    -> pool_control::CleanupOutcome<&'static str> {
    let cleanup = pool_control::CleanupEvidence::new();
    let result = async {
    s.serving_accounts.require(&body.account_id)
        .map_err(operation_ownership::completed_preflight)?;
    let _management = cleanup.outcome(pool_control::management_observed(&s, body.account_id, true).await)?;
    // Mint + signin happen INSIDE the account's browser: api.framer.com
    // validates request/environment consistency (Origin/UA/TLS vs the Castle
    // token's fingerprint) and 400s the bare-HTTP-client variant — observed.
    use framer_core::observability as obs;
    let t0 = std::time::Instant::now();
    let req_id = uuid::Uuid::new_v4().to_string();
    let r = _management.observe_result(cleanup.external(
        s.browser.auth_begin(&body.account_id, &body.email).await));
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
    r.map_err(ApiError::Internal)?;
    Ok("sent")
    }.await;
    cleanup.finish(result)
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
async fn complete_login(state: State<AppState>, body: Json<CompleteLoginBody>) -> Result<Json<serde_json::Value>, ApiError> {
    complete_login_observed(state, body).await.result
}

async fn complete_login_observed(State(s): State<AppState>, Json(body): Json<CompleteLoginBody>)
    -> pool_control::CleanupOutcome<Json<serde_json::Value>> {
    let cleanup = pool_control::CleanupEvidence::new();
    let result = async {
    s.serving_accounts.require(&body.account_id)
        .map_err(operation_ownership::completed_preflight)?;
    let _management = cleanup.outcome(pool_control::management_observed(&s, body.account_id, true).await)?;
    // In-page token exchange; returns the session cookie read out of the
    // browser jar via CDP plus the in-page /web/users/me payload.
    use framer_core::observability as obs;
    let t0 = std::time::Instant::now();
    let req_id = uuid::Uuid::new_v4().to_string();
    let r = s
        .browser
        .auth_complete(&body.account_id, &body.email_token)
        .await;
    let r = _management.observe_result(cleanup.external(r));
    let mut ev = obs::Event::new("login.complete")
        .request_id(&req_id)
        .account(&body.account_id)
        .duration_ms(obs::elapsed_ms(t0));
    ev = match &r {
        Ok(_) => ev.outcome("ok"),
        Err(e) => ev.error(e),
    };
    ev.emit();
    let (cookie_header, me) = r.map_err(ApiError::Internal)?;
    let user_id = me["id"]
        .as_str()
        .ok_or_else(|| ApiError::Internal("bad /web/users/me response".into()))?;
    let session = framer_core::port::Session {
        active_user: framer_core::ids::FramerUserId(user_id.to_string()),
        cookie_header,
    };
    let spaces = me["spaces"].as_array().cloned().unwrap_or_default();
    let first_org = spaces.iter().find(|sp| sp["scope"].as_str() == Some("organization")).cloned();
    let team = first_org
        .as_ref()
        .and_then(|sp| sp["ownerId"].as_str())
        .map(|t| framer_core::ids::TeamId(t.to_string()));

    let mut account = s
        .state
        .get(body.account_id)
        .await
        .map_err(ApiError::Internal)?
        .unwrap_or_else(|| ManagedAccount {
            id: body.account_id,
            alias: body.alias.clone().unwrap_or_else(|| "account".into()),
            email: None,
            framer_user_id: session.active_user.clone(),
            castle_device_id: uuid::Uuid::new_v4().to_string(),
            proxy: None,
            current_team: None,
            current_project: None,
            extra_workspaces: Vec::new(),
            state: framer_core::account::AccountState::Ready,
            credit: None,
            consecutive_failures: 0,
            cooldown_step: 0,
            cooling_until_ms: None,
            cooldown_cause: None,
        });
    let registration = s.serving_accounts.registration.lock().await;
    // Dedupe by email BEFORE anything else: adding the same account twice
    // splits its credit ledger across two records and makes rotation think it
    // has more capacity than it does.
    if let Some(raw) = body.email.as_deref() {
        let email = framer_core::account::normalize_email(raw);
        let pool = s.state.list().await.map_err(ApiError::Internal)?;
        if let Some(dup) = pool
            .iter()
            .find(|a| a.id != account.id && a.email.as_deref() == Some(email.as_str()))
        {
            return Err(ApiError::Internal(format!(
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
    s.sessions.save(&session).await.map_err(ApiError::Internal)?;

    // Claim the second workspace's 500-credit grant BEFORE admission scans
    // (first TWO workspaces each get one, verified 2026-10-05). Best-effort.
    {
        let _ = cleanup.external(s.client.claim_workspace_grant(&session).await);
    }
    if account.framer_user_id != session.active_user {
        account.extra_workspaces.clear();
        if team.is_none() {
            account.current_team = None;
            account.current_project = None;
        }
    }
    account.framer_user_id = session.active_user.clone();
    account.state = framer_core::account::AccountState::Ready;
    if let Some(a) = &body.alias {
        account.alias = a.clone();
    }
    if team.is_some() {
        if account.current_team != team {
            account.current_project = None;
        }
        account.current_team = team;
    }
    if let Some(pid) = &body.project_id {
        account.current_project = Some(framer_core::ids::ProjectId(pid.clone()));
    }
    s.state.upsert(&account).await.map_err(ApiError::Internal)?;
    drop(registration);

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
        account_updates: &s.serving_accounts.health_update,
    };
    let claimed = claimed_workspaces(&s, account.id).await?;
    let admission =
        cleanup.core(framer_core::credits::admit(&deps, &mut account, &session, &claimed).await);
    if let Err(e) = admission {
        if matches!(&e, framer_core::error::CoreError::NotEligible(_)) {
            account_scope_publication::mark_refused(&s, &mut account).await?;
        }
        return Err(ApiError::Internal(e.to_string()));
    }

    let expected_scope = account.clone();
    // Project: adopt the workspace's newest, else CREATE one — every AI call
    // needs a projectId and a fresh workspace has none.
    if account.current_project.is_none() {
        if let Some(team) = account.current_team.clone() {
            match cleanup.external(s.client.team_projects(&session, &team).await) {
                Ok(projects) => {
                    if let Some(first) = projects.into_iter().next() {
                        account.current_project = Some(first);
                    }
                }
                Err(e) => eprintln!("[login] project listing failed: {e}"),
            }
            if account.current_project.is_none() {
                match cleanup.external(s.client.create_project(&session, &team).await) {
                    Ok(p) => {
                        eprintln!("[login] created project {}", p.0);
                        account.current_project = Some(p);
                    }
                    Err(e) => eprintln!("[login] project creation failed (non-fatal): {e}"),
                }
            }
        }
    }
    // Multi-workspace goose: claim every other credited workspace as a rotated
    // scope (each carries its own 429 budget).
    populate_extra_scopes(&s, &session, &mut account, &claimed, &cleanup).await;
    account_scope_publication::publish(&s, &expected_scope, &mut account).await?;

    Ok(Json(serde_json::json!({
        "status": "signed-in",
        "team": account.current_team.as_ref().map(|t| t.0.clone()),
        "credits": account.credit.as_ref().map(|c| c.available),
        "account_id": account.id.0,
        "framer_user_id": account.framer_user_id.0,
        "team": account.current_team.map(|t| t.0),
        "project": account.current_project.map(|p| p.0),
    })))
    }.await;
    cleanup.finish(result)
}

#[derive(Deserialize)]
struct ImportBody {
    account_id: AccountId,
    /// Refuse a session for any other login before saving or provisioning it.
    expected_framer_user_id: Option<String>,
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
    /// Pin the account to a specific workspace (team). Without it the first
    /// org is used — wrong when an account has multiple workspaces and the
    /// credited one isn't first (2026-10-05: workspace 2 grants are real).
    team_id: Option<String>,
}

/// Import an externally-minted session cookie: verify it against
/// /web/users/me, then register the account exactly like complete_login.
async fn import_account(state: State<AppState>, body: Json<ImportBody>)
    -> Result<Json<serde_json::Value>, ApiError> {
    import_account_observed(state, body).await.result
}

async fn import_account_observed(State(s): State<AppState>, Json(body): Json<ImportBody>)
    -> pool_control::CleanupOutcome<Json<serde_json::Value>> {
    let cleanup = pool_control::CleanupEvidence::new();
    let result = async {
    if body.expected_framer_user_id.as_ref()
        .is_some_and(|user| user.trim().is_empty() || user.len() > 128) {
        return Err(operation_ownership::completed_preflight(
            ApiError::InvalidRequest("expected Framer user is invalid".into())));
    }
    let management = cleanup.outcome(pool_control::management_observed(&s, body.account_id, false).await)?;
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
    let me = cleanup.external(s.client.me(&probe).await).map_err(ApiError::Internal)?;
    let user_id = me["id"]
        .as_str()
        .ok_or_else(|| ApiError::Internal("bad /web/users/me response (cookie invalid?)".into()))?;
    if body.expected_framer_user_id.as_deref()
        .is_some_and(|expected| expected != user_id) {
        return Err(ApiError::InvalidRequest("session belongs to a different Framer user".into()));
    }
    let session = framer_core::port::Session {
        active_user: framer_core::ids::FramerUserId(user_id.to_string()),
        cookie_header,
    };
    let spaces = me["spaces"].as_array().cloned().unwrap_or_default();
    let first_org = if let Some(tid) = &body.team_id {
        spaces.iter().find(|sp| sp["ownerId"].as_str() == Some(tid.as_str())).cloned()
    } else {
        spaces.iter().find(|sp| sp["scope"].as_str() == Some("organization")).cloned()
    };
    let team = first_org
        .as_ref()
        .and_then(|sp| sp["ownerId"].as_str())
        .map(|t| framer_core::ids::TeamId(t.to_string()));

    let mut account = s
        .state
        .get(body.account_id)
        .await
        .map_err(ApiError::Internal)?
        .unwrap_or_else(|| ManagedAccount {
            id: body.account_id,
            alias: body.alias.clone().unwrap_or_else(|| "imported".into()),
            email: None,
            framer_user_id: session.active_user.clone(),
            castle_device_id: uuid::Uuid::new_v4().to_string(),
            proxy: None,
            current_team: None,
            current_project: None,
            extra_workspaces: Vec::new(),
            state: framer_core::account::AccountState::Ready,
            credit: None,
            consecutive_failures: 0,
            cooldown_step: 0,
            cooling_until_ms: None,
            cooldown_cause: None,
        });
    let registration = s.serving_accounts.registration.lock().await;
    // Same dedupe + naming rule as complete_login: one email, one account.
    if let Some(raw) = body.email.as_deref() {
        let email = framer_core::account::normalize_email(raw);
        let pool = s.state.list().await.map_err(ApiError::Internal)?;
        if let Some(dup) = pool
            .iter()
            .find(|a| a.id != account.id && a.email.as_deref() == Some(email.as_str()))
        {
            return Err(ApiError::Internal(format!(
                "{email} is already added as account {} — not adding it twice",
                dup.alias,
            )));
        }
        if body.alias.is_none() {
            account.alias = email.clone();
        }
        account.email = Some(email);
    }
    s.sessions.save(&session).await.map_err(ApiError::Internal)?;

    // Claim the second workspace's 500-credit grant BEFORE admission scans
    // (first TWO workspaces each get one, verified 2026-10-05). Best-effort:
    // admission below decides usability either way.
    if body.team_id.is_none() {
        let _ = cleanup.external(s.client.claim_workspace_grant(&session).await);
    }

    if account.framer_user_id != session.active_user {
        account.extra_workspaces.clear();
        if team.is_none() {
            account.current_team = None;
            account.current_project = None;
        }
    }
    account.framer_user_id = session.active_user.clone();
    account.state = framer_core::account::AccountState::Ready;
    if let Some(a) = &body.alias {
        account.alias = a.clone();
    }
    if team.is_some() {
        if account.current_team != team {
            account.current_project = None;
        }
        account.current_team = team;
    }
    if let Some(pid) = &body.project_id {
        account.current_project = Some(framer_core::ids::ProjectId(pid.clone()));
    }
    if let Some(px) = &body.proxy {
        if account.proxy.as_ref() != Some(px) {
            cleanup.outcome(s.serving_accounts.park_managed_observed(s.browser.as_ref(), body.account_id).await)?;
        }
        account.proxy = Some(px.clone());
    }
    cleanup.external(s.browser.register_account_proxy(&body.account_id, account.proxy.clone()).await)
        .map_err(ApiError::Internal)?;
    s.state.upsert(&account).await.map_err(ApiError::Internal)?;

    drop(registration);
    // Admission (same rule as complete_login): an account with no credited
    // workspace cannot serve a request, so refuse it rather than let rotation
    // discover that one failure at a time.
    {
        let deps = Deps {
            client: s.client.as_ref(),
            browser: s.browser.as_ref(),
            sessions: s.sessions.as_ref(),
            state: s.state.as_ref(),
            account_updates: &s.serving_accounts.health_update,
        };
        let claimed = claimed_workspaces(&s, account.id).await?;
        if let Err(e) =
            cleanup.core(framer_core::credits::admit(&deps, &mut account, &session, &claimed).await)
        {
            if matches!(&e, framer_core::error::CoreError::NotEligible(_)) {
                account_scope_publication::mark_refused(&s, &mut account).await?;
            }
            return Err(ApiError::Internal(e.to_string()));
        }
        let mut expected_scope = account.clone();
        // Adopt an existing project if the account has one. Every AI call needs
        // a projectId, so an imported account without one is admitted but
        // unusable -- and was previously mistaken for dead by the dedupe sweep.
        if account.current_project.is_none() {
            if let Some(team) = account.current_team.clone() {
                match cleanup.external(s.client.team_projects(&session, &team).await) {
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
        // Still no project? CREATE one in the admitted workspace (a fresh
        // workspace has none, and every AI call needs a projectId).
        if account.current_project.is_none() {
            if let Some(team) = account.current_team.clone() {
                match cleanup.external(s.client.create_project(&session, &team).await) {
                    Ok(p) => {
                        eprintln!("[import] {}: created project {}", account.alias, p.0);
                        account.current_project = Some(p);
                        if s.state.upsert(&account).await.is_ok() {
                            expected_scope = account.clone();
                        }
                    }
                    Err(e) => eprintln!("[import] project creation failed (non-fatal): {e}"),
                }
            }
        }
        // Multi-workspace goose: claim every other credited workspace as a
        // rotated scope (each carries its own 429 budget).
        populate_extra_scopes(&s, &session, &mut account, &claimed, &cleanup).await;
        account_scope_publication::publish(&s, &expected_scope, &mut account).await?;
    }

    // Release account mutation ownership before an independent warmup acquires it.
    drop(management);
    // Excluded records retain their saved session and registered proxy without
    // launching a browser. Later activation uses the same admission checks.
    let prewarm_allowed = s.serving_accounts.allows(&body.account_id);
    // Native imports retain device admission; they do not initialize a browser.
    if prewarm_allowed {
        let browser = s.browser.clone();
        let state = s.clone();
        let id = body.account_id;
        let prewarm = operation_ownership::spawn_with_cleanup(s.operation_ownership.as_ref(),
            framer_core::operation_gate::OperationKind::Background, async move {
            let acquired = pool_control::lease_account_observed(&state, id).await;
            let lease = match acquired.result {
                Ok(lease) => lease,
                Err(_) => return ((), acquired.cleanup_confirmed),
            };
            if let Err(error) = current_account(&state, id).await {
                return ((), acquired.cleanup_confirmed && matches!(error,
                    ApiError::Cooling { .. } | ApiError::AccountCooling { .. }
                    | ApiError::ServingCapacity(_)));
            }
            let t0 = std::time::Instant::now();
            let r = browser.castle_token(&id).await;
            let confirmed = acquired.cleanup_confirmed
                && !matches!(&r, Err(error) if pool_control::cleanup_unconfirmed(error));
            let r = pool_control::observe_lease_result(&lease, r);
            framer_core::observability::Event::new("account.prewarm")
                .account(&id)
                .duration_ms(framer_core::observability::elapsed_ms(t0))
                .outcome(if r.is_ok() { "ok" } else { "error" })
                .field("cause", r.as_ref().err().map(|error| framer_core::error::safe_failure_summary(error)))
                .emit();
            ((), confirmed)
        });
        if let Err(error) = prewarm {
            framer_core::observability::Event::new("account.prewarm")
                .account(&id).outcome("deferred")
                .field("reason", "ownership_admission")
                .field("cause", operation_ownership::admission_message(error)).emit();
        }
    }
    Ok(Json(serde_json::json!({
        "status": "imported",
        "serving_allowed": s.serving_accounts.allows(&body.account_id),
        "account_id": account.id.0,
        "framer_user_id": account.framer_user_id.0,
        "team": account.current_team.map(|t| t.0),
        "project": account.current_project.map(|p| p.0),
    })))
    }.await;
    cleanup.finish(result)
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

async fn run_request(
    state: State<AppState>,
    body: Result<Json<RunBody>, JsonRejection>,
) -> Result<axum::response::Response, ApiError> {
    match body {
        Ok(body) => run(state, body).await,
        Err(error) => Ok(operation_ownership::completed_response(
            axum::response::IntoResponse::into_response(error), true)),
    }
}

async fn run(
    State(s): State<AppState>,
    Json(body): Json<RunBody>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    s.serving_accounts.require(&body.account_id)
        .map_err(operation_ownership::completed_preflight)?;
    let acquired = pool_control::lease_account_observed(&s, body.account_id).await;
    let mut cleanup_confirmed = acquired.cleanup_confirmed;
    let residency = match acquired.result {
        Ok(lease) => lease,
        Err(error) => return Ok(operation_ownership::completed_response(
            error.into_response(), acquired.cleanup_confirmed)),
    };
    let mut account = match current_account(&s, body.account_id).await {
        Ok(account) => account,
        Err(error) => {
            let confirmed = cleanup_confirmed && matches!(&error, ApiError::Cooling { .. }
                | ApiError::AccountCooling { .. } | ApiError::ServingCapacity(_));
            return Ok(operation_ownership::completed_response(error.into_response(), confirmed));
        }
    };

    let deps = Deps {
        client: s.client.as_ref(),
        browser: s.browser.as_ref(),
        sessions: s.sessions.as_ref(),
        state: s.state.as_ref(),
        account_updates: &s.serving_accounts.health_update,
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
    let outcome = pipeline::run(&deps, &mut account, &req).await;
    if matches!(&outcome, Err(framer_core::error::CoreError::Store(_))) {
        cleanup_confirmed = false;
    }
    if let Err(framer_core::error::CoreError::Framer(error)) = &outcome {
        if pool_control::cleanup_unconfirmed(error) {
            cleanup_confirmed = false;
            operation_ownership::record_cleanup_failure(error);
            if let Err(error) = residency.mark_unconfirmed_with_cause(error) {
                return Ok(operation_ownership::completed_response(
                    ApiError::PoolBusy(error.to_string()).into_response(), false));
            }
        }
    }
    let response: Result<Json<InferenceOutcome>, ApiError> = outcome.map(Json)
        .map_err(|error| match error {
            framer_core::error::CoreError::AccountCooling(cooldown, cause) =>
                ApiError::AccountCooling { retry_after_secs: cooldown.retry_after_secs, cause },
            error @ framer_core::error::CoreError::AccountNotReady(_) =>
                ApiError::ServingCapacity(error.to_string()),
            error => ApiError::Internal(error.to_string()),
        });
    Ok(operation_ownership::completed_response(response.into_response(), cleanup_confirmed))
}

#[derive(Debug)]
enum ApiError {
    Internal(String),
    InsufficientCredits { available: f64, floor: f64 },
    InvalidRequest(String),
    PoolBusy(String),
    ServingCapacity(String),
    Cooling { retry_after_secs: u64 },
    AccountCooling { retry_after_secs: u64, cause: String },
}

impl ApiError {
    fn diagnostic(&self) -> String {
        match self {
            Self::InsufficientCredits { available, floor } => format!(
                "The selected workspace has {available:.2} AI credits. This metered route requires at least {floor:.2}. Restore credits on the same workspace before retrying."),
            Self::Internal(value) | Self::InvalidRequest(value) | Self::PoolBusy(value)
                | Self::ServingCapacity(value) => framer_core::error::safe_failure_summary(value),
            Self::Cooling { retry_after_secs } => format!(
                "security refusals triggered a global pause with {retry_after_secs} seconds remaining"),
            Self::AccountCooling { retry_after_secs, cause } => format!(
                "{}; account cooldown has {retry_after_secs} seconds remaining",
                framer_core::error::safe_failure_summary(cause)),
        }
    }
    fn operation_admission(error: framer_core::operation_gate::GateError) -> Self {
        Self::ServingCapacity(operation_ownership::admission_message(error))
    }
}

/// Dialect-aware API error: correct HTTP status + client-shaped error JSON, so
/// clients see the REAL upstream cause (context overflow → compact, not a
/// retry-storm on a generic 500).
struct ShapedApiError(StatusCode, serde_json::Value);

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let account_cause = match &self {
            Self::AccountCooling { cause, .. } =>
                Some(framer_core::error::safe_failure_summary(cause)),
            _ => None,
        };
        match self {
            error @ Self::InsufficientCredits { .. } => ShapedApiError(
                StatusCode::PAYMENT_REQUIRED,
                serde_json::json!({"error":{"type":"insufficient_quota", "code":"insufficient_quota",
                    "message":error.diagnostic(), "retryable":false}}),
            ).into_response(),
            Self::InvalidRequest(message) => (StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":{"code":"invalid_request","message":message}}))).into_response(),
            Self::PoolBusy(message) => (StatusCode::SERVICE_UNAVAILABLE,
                [(axum::http::header::RETRY_AFTER, "1")],
                Json(serde_json::json!({"error":{"code":"framerize_pool_busy",
                    "message":message,"retry_after_seconds":1}}))).into_response(),
            Self::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
            Self::ServingCapacity(message) => (StatusCode::SERVICE_UNAVAILABLE, message).into_response(),
            Self::Cooling { retry_after_secs } | Self::AccountCooling { retry_after_secs, .. } => {
                // Rechecks can see recovered capacity or an in-flight success.
                // Their interval never shortens the underlying cooldown.
                let recheck_after_secs = retry_after_secs.clamp(1, 30);
                let (code, reason) = if let Some(cause) = account_cause {
                    ("framerize_account_cooling", format!("Cannot start the request because {cause}."))
                } else {
                    ("framerize_cooling", "Recent security refusals across multiple accounts triggered the global admission pause.".to_owned())
                };
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    [(axum::http::header::RETRY_AFTER, recheck_after_secs.to_string())],
                    Json(serde_json::json!({
                        "type": "error",
                        "error": {
                            "type": "overloaded_error",
                            "code": code,
                            "message": format!("{reason} Retry in {recheck_after_secs} seconds."),
                            "retry_after_seconds": recheck_after_secs,
                            "cooldown_remaining_seconds": retry_after_secs
                        }
                    })),
                ).into_response()
            }
        }
    }
}

impl axum::response::IntoResponse for ShapedApiError {
    fn into_response(self) -> axum::response::Response {
        let unreplayable = self.0 == StatusCode::PAYMENT_REQUIRED
            || self.1.pointer("/error/code").and_then(|value| value.as_str())
            == Some(framer_core::error::ADAPTIVE_INCOMPLETE_CODE);
        let mut response = (
            self.0,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            serde_json::to_string(&self.1).unwrap_or_else(|_| "{}".into()),
        ).into_response();
        if unreplayable {
            response.headers_mut().insert("x-should-retry",
                axum::http::HeaderValue::from_static("false"));
        }
        response
    }
}

#[cfg(test)]
mod pause_recovery_tests;

#[cfg(test)]
mod adaptive_retry_tests;

#[cfg(test)]
mod credit_error_tests;
