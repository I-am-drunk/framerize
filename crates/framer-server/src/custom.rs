//! Custom-route HTTP surface: POST /api/custom/complete.
//!
//! Thin shell over the framer-custom engine. The engine's only upstream
//! contact is CustomTransport. Mock mode (FRAMERIZE_CUSTOM_TRANSPORT=mock)
//! runs a scripted transport for route-level checks and the tests below.
//! Real mode (default) serves the live page stream (see REAL_MODE).
//!
//! REAL_MODE (mirrors native_complete + proxy::attempt exactly, no invention):
//! per request the handler assembles, in order,
//! (a) serving account via serving_accounts.select (x-framerize-account pin
//!     or least-failed eligible) + lease_account residency + current_account
//!     admission + wave check, mirroring native_complete;
//! (b) session via sessions.load + project/team scope via pick_scope,
//!     mirroring native_complete (one account spans N workspace budgets);
//! (c) the AI bearer JWT via FramerClient::custom_bearer (the same
//!     per-account cached mint proxy::forward uses), with the same one-retry
//!     eviction (custom_bearer_fresh) on a 401 open;
//! (d) the Gq envelope via framer_api::proxy::envelope -- the single
//!     constructor attempt() itself calls, so leg payloads wrap
//!     byte-identically (project/category/name/model/step/session/
//!     step-index/turn/userPrompt + request = native payload verbatim);
//! (e) a HostPageStream over a BridgePageEval adapter (Arc<dyn
//!     BrowserBridge>::eval_small -- one small eval per call, the
//!     per-account Mutex never held across the stream, ai_post untouched).
//! The engine drives legs through an EnvelopePageStream that injects
//! model + stream:true into each leg's native payload and wraps it in the
//! envelope before open_stream. Gate (per workspace x model), residency
//! observation, and account-health recording all mirror native_complete.

use super::{current_account, hdr, pick_scope, record_account_health, stable_session, ApiError, AppState};
use super::pool_control::{lease_account, RequestResidency};
use async_trait::async_trait;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use framer_core::ids::{AccountId, ProjectId};
use framer_core::port::{BrowserBridge, Session};
use serde::Deserialize;
use std::sync::Arc;

pub const TRANSPORT_ENV: &str = "FRAMERIZE_CUSTOM_TRANSPORT";

/// Per-stage budgets for the real-mode dep assembly. Every one of these
/// awaits was unbounded; a stall in any of them used to wedge the request
/// with no log line until the client gave up. Each stage now fails loud with
/// the request id so the next hang report names its stage.
const STAGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
/// Whole-request budget for the engine run. Must stay under the tightest
/// client timeout so the response is always typed.
const ENGINE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(540);

/// Upstream dialect for custom-route legs: the engine emits OpenAI
/// chat-shaped `{messages: [...]}` payloads, so legs ride /ai/v3/chat/
/// (OpenRouterChat), the same route freechat's chat-shaped chunks use.
/// Default upstream dialect for custom legs. Direct-route models (the sol
/// roster: they exist ONLY on /ai/v3/openai/responses/, metered) switch the
/// leg to that route instead -- see custom_real.
const DIALECT: framer_api::proxy::Dialect = framer_api::proxy::Dialect::OpenRouterChat;
/// Step tag for the envelope: freechat's rule (chain_step) -- real
/// translation traffic uses step=translate under category=translation,
/// message otherwise (invisibility audit 2026-10-11).
fn chain_step(category: &str) -> &'static str {
    if category == "translation" {
        "translate"
    } else {
        "message"
    }
}

pub fn custom_routes(state: AppState) -> Router {
    Router::new()
        .route("/complete", post(custom_complete_handler))
        .route(
            "/complete/",
            post(custom_complete_handler).get(hint_handler).put(hint_handler),
        )
        .route("/complete/status", axum::routing::get(status_handler))
        .with_state(state)
}

/// Loud hint for wrong-method/wrong-path callers (T3 compat): tells them
/// the exact working shape instead of a bare 404/405.
async fn hint_handler() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(serde_json::json!({
            "error": "use POST /api/custom/complete with JSON {prompt, model, max_legs}",
        })),
    )
        .into_response()
}

/// Liveness for the route behind the handler (the daemon /api/health only
/// proves the listener; this proves the custom handler is mounted).
async fn status_handler() -> Response {
    Json(serde_json::json!({"custom_route": "ok"})).into_response()
}

#[derive(Deserialize)]
struct CompleteBody {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    model: String,
    /// Max accepted legs for this request (default 8). The live ledger run
    /// caps it at 1 for a minimal single-leg proof.
    #[serde(default)]
    max_legs: Option<u32>,
    /// Per-leg stop override (events). Operator/testing knob: probing the
    /// end-of-stream cutoff needs a precise stop_after.
    #[serde(default)]
    stop: Option<u32>,
}

fn mock_script() -> Vec<framer_custom::Event> {
    vec![
        framer_custom::Event::Delta("The custom route streams its first prefix here, ".into()),
        framer_custom::Event::Delta("and the second delta carries it past the finish bar. ".into()),
        framer_custom::Event::Done,
    ]
}

/// PageEval over the shared trait-object bridge: clones the Arc from
/// AppState, so the stream is fully owned (no lifetimes, no downcast --
/// AppState holds Arc<dyn BrowserBridge> and no downcast exists anywhere).
struct BridgePageEval {
    browser: Arc<dyn BrowserBridge>,
}

#[async_trait]
impl framer_host::page_stream::PageEval for BridgePageEval {
    async fn eval_small(
        &self,
        account: &AccountId,
        expr: String,
    ) -> Result<serde_json::Value, String> {
        self.browser.eval_small(account, &expr).await
    }
}

/// Per-request deps for one real-mode leg stream (mirrors native_complete's
/// assembly: account/session/scope + bearer + envelope context).
struct RealDeps {
    account: AccountId,
    session: Session,
    project: ProjectId,
    bearer: String,
    category: String,
    step: String,
    chat_session: String,
    path: &'static str,
}

/// PageStream that wraps each engine leg payload in the Gq envelope before
/// handing it to the host stream. The envelope MUST carry stream:true in the
/// native request: streaming legs settle only at the usage/terminal frame
/// (which the page abort never lets arrive); non-streaming legs bill at
/// dispatch and no abort can un-bill them. The engine emits bare native payloads
/// ({messages:[...]}, no model/stream); the envelope needs model (top-level
/// gf() mirror) + stream:true, then envelope() builds the exact body
/// attempt() would send. On a 401 open the bearer is evicted + reminted
/// once and the open retried (mirrors forward's one-retry exactly).
struct EnvelopePageStream {
    inner: framer_host::page_stream::HostPageStream<BridgePageEval>,
    deps: RealDeps,
    client: Arc<dyn framer_core::port::FramerClient>,
    model: String,
    /// Kept so the 401-retry path rebuilds the host stream exactly.
    browser: Arc<dyn BrowserBridge>,
    team: String,
    /// Every turnId this stream opened, in order. The chain needs them for
    /// SETTLE REVERSAL: /ai/v3/refund/ reverses a settled turn exactly and
    /// is a denied no-op on an unsettled one, so a chain whose ledger moved
    /// refunds its opened turns until the balance recovers.
    opened_turns: Arc<std::sync::Mutex<Vec<String>>>,
}

/// CUSTOM-ROUTE-ONLY refusal marks (2026-11-09): a Castle refusal on the
/// in-page open is specific to the OPEN pattern (ambient-page mint + fetch)
/// -- the same account often serves ai_post fine from its device contexts
/// (measured live: e9d59f78 refused two custom opens while its regular-route
/// traffic returned 200s). So it must NOT cool the account (rotation.rs)
/// and must NOT feed the wave guard: that would park healthy serving. Mark
/// it here instead: the custom route skips the marked account for the TTL
/// while every other route keeps using it.
static OPEN_REFUSAL: std::sync::Mutex<
    Option<std::collections::HashMap<String, std::time::Instant>>,
> = std::sync::Mutex::new(None);
const OPEN_REFUSAL_TTL: std::time::Duration = std::time::Duration::from_secs(600);

fn mark_open_refused(account: &str) {
    if let Ok(mut guard) = OPEN_REFUSAL.lock() {
        guard
            .get_or_insert_with(Default::default)
            .insert(account.to_string(), std::time::Instant::now());
    }
}

fn open_refusal_active(account: &str) -> bool {
    if let Ok(mut guard) = OPEN_REFUSAL.lock() {
        if let Some(map) = guard.as_mut() {
            if let Some(at) = map.get(account) {
                if at.elapsed() < OPEN_REFUSAL_TTL {
                    return true;
                }
                map.remove(account);
            }
        }
    }
    false
}

/// RUNWAY RULE (2026-11-09): the in-page abort is deterministic only while
/// the server is still generating when it lands. Generation ends at the
/// upstream token cap (or a model early-stop). One SSE data line carries AT
/// MOST one token of content -- empty lines count as stop events too -- so
/// the stop fires at <= stop_after GENERATED tokens. Requiring the cap to
/// be at least RUNWAY x stop_after guarantees the stream is mid-generation
/// when ctl.abort() fires, no matter how fast the model is: the race cannot
/// be lost to speed. The only residual race is a model early-stop below
/// stop_after (a complete short answer); the per-leg raced flag + the
/// ledger guard still catch that, and RED ALERT stops the chain on any
/// settle. The cap costs nothing: the abort means the runway tokens are
/// never generated.
const LEG_CAP_RUNWAY: u64 = 3;
/// Bound the worst-case settle if a leg ever races anyway.
const LEG_CAP_MAX: u64 = 16_000;

fn leg_cap_with_runway(caller: Option<u64>, stop_after: u32) -> u64 {
    let floor = (stop_after as u64).saturating_mul(LEG_CAP_RUNWAY).max(64);
    caller.unwrap_or(4_000).max(floor).min(LEG_CAP_MAX)
}

impl EnvelopePageStream {
    fn wrap(&self, payload: &serde_json::Value, stop_after: u32) -> serde_json::Value {
        // The direct Responses route takes {model, input, stream}, not
        // {messages}: reshape the engine's chat-shaped leg. The leg's user
        // content is the whole instruction; input is that string.
        let mut native = if self.deps.path.contains("responses") {
            let text = payload["messages"]
                .as_array()
                .and_then(|ms| ms.iter().rev().find(|m| m["role"] == "user"))
                .and_then(|m| m["content"].as_str())
                .unwrap_or("")
                .to_string();
            serde_json::json!({
                "input": text,
            })
        } else {
            payload.clone()
        };
        // T3 sends bare chat shape (no stream, no max_tokens, sometimes a
        // prefixed model id). Normalize EVERYTHING the meter keys on here,
        // at the boundary, so no caller shape can silently downgrade a leg
        // to the bill-at-dispatch path (proven by the instant-402s: the
        // debit arrives WITH the open, and no abort can un-bill it).
        // Gateway form is UNPREFIXED (see custom_real model rule above).
        let model = self.model.clone();
        let model = model.rsplit('/').next().unwrap_or(&model).to_string();
        native["model"] = serde_json::Value::String(model);
        // Token cap name per route: the direct Responses route takes
        // max_output_tokens (and 400s "Unknown parameter: 'max_tokens'");
        // the chat route takes max_tokens. Either way the cap obeys the
        // RUNWAY RULE (leg_cap_with_runway): never below 3x stop_after, so a
        // fast model cannot reach the terminal before the in-page abort.
        let cap_key = if self.deps.path.contains("responses") {
            "max_output_tokens"
        } else {
            "max_tokens"
        };
        let caller_cap = native.get(cap_key).and_then(|v| v.as_u64());
        native[cap_key] =
            serde_json::Value::Number(serde_json::Number::from(leg_cap_with_runway(caller_cap, stop_after)));
        // STREAMING keeps the leg on the metered path the page abort guards:
        // the stop fires mid-stream before the usage/terminal frame, so the
        // leg never settles. stream:false legs bill at dispatch (the meter
        // settles the full leg price up front -- proven by the 402s above,
        // where the debit arrived WITH the open, not after it). Never
        // downgrade this: the whole $0 mechanism depends on it.
        native["stream"] = serde_json::Value::Bool(true);
        let env = framer_api::proxy::envelope(
            &self.deps.project,
            &self.deps.category,
            &self.deps.step,
            &native,
            &self.deps.chat_session,
            0,
        );
        if let Some(turn) = env["turnId"].as_str() {
            if let Ok(mut turns) = self.opened_turns.lock() {
                turns.push(turn.to_string());
            }
        }
        env
    }

    fn is_unauthorized(err: &str) -> bool {
        // HostPageStream open errors read "page stream open failed: http
        // {status}: ..."; 401 = the cached JWT died mid-flight.
        err.contains("http 401")
    }
}

#[async_trait]
impl framer_custom::PageStream for EnvelopePageStream {
    async fn open_stream(
        &mut self,
        payload: &serde_json::Value,
        dialect: framer_custom::Dialect,
        stop_after: u32,
    ) -> Result<String, String> {
        let body = self.wrap(payload, stop_after);
        match self.inner.open_stream(&body, dialect, stop_after).await {
            Err(e) if Self::is_unauthorized(&e) => {
                // One retry with a fresh JWT (mirrors forward): evict the
                // dead token, remint, rebuild the stream bearer, retry once.
                let fresh = self
                    .client
                    .custom_bearer_fresh(&self.deps.session, &self.deps.account)
                    .await?;
                // If the 401 was a dead cookie rather than a dead JWT, the
                // fresh bearer alone would 401 again: re-seed the jar before
                // retrying (same call attempt() makes before every ai_post).
                // Best-effort: a seed failure surfaces with the retry error.
                let seed_err = self
                    .browser
                    .seed_session(&self.deps.account, &self.deps.session.cookie_header)
                    .await
                    .err();
                if let Some(se) = seed_err {
                    return Err(format!("custom 401 retry: re-seed failed ({se}); bearer refreshed"));
                }
                self.deps.bearer = fresh.clone();
                self.inner = framer_host::page_stream::HostPageStream::new(
                    Arc::new(BridgePageEval {
                        browser: self.browser.clone(),
                    }),
                    self.deps.account,
                    fresh,
                    self.team.clone(),
                    self.deps.path.to_string(),
                );
                let body = self.wrap(payload, stop_after);
                self.inner.open_stream(&body, dialect, stop_after).await
            }
            other => other,
        }
    }

    async fn drain(&mut self, stream_id: &str) -> Result<framer_custom::StreamBatch, String> {
        self.inner.drain(stream_id).await
    }

    async fn cancel(&mut self, stream_id: &str) -> Result<bool, String> {
        self.inner.cancel(stream_id).await
    }

    async fn credits(&self) -> Result<f64, String> {
        self.inner.credits().await
    }
}

async fn custom_complete_handler(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CompleteBody>,
) -> Response {
    if body.prompt.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "prompt is required"})),
        )
            .into_response();
    }
    let mode = std::env::var(TRANSPORT_ENV).unwrap_or_else(|_| "real".into());
    if mode == "mock" {
        return mock_complete(&body).await;
    }
    match custom_real(s, &headers, &body).await {
        Ok(resp) => resp,
        Err(e) => e.into_response(),
    }
}

async fn mock_complete(body: &CompleteBody) -> Response {
    let _model = body.model.clone();
    let mut transport = framer_custom::mock_util::ScriptedTransport::new(mock_script(), 100.0);
    let settings = framer_custom::ChainSettings::default();
    let assembled = match framer_custom::complete(&mut transport, &body.prompt, &settings).await {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
                .into_response();
        }
    };
    sse_response(&assembled, "mock", 0)
}

fn sse_response(assembled: &framer_custom::Assembled, req_id: &str, refunded: u32) -> Response {
    // Frame contract: text+legs are the stable fields every client reads.
    // outcome/total_debit/legs-detail/req_id are additive diagnostics so a
    // billed, short, or raced run is diagnosable from the response alone
    // (mirrors the [custom-route] daemon log lines, same req_id).
    let event = format!(
        "data: {}\n\n",
        serde_json::json!({
            "text": assembled.text,
            "legs": assembled.legs.len(),
            "outcome": format!("{:?}", assembled.outcome),
            "total_debit": assembled.total_debit,
            // Deterministic progress + the compaction input: per-leg captured
            // chars and an estimated output-token count. The abort lands
            // BEFORE the usage frame by design, so real token counts never
            // arrive -- the estimate (chars/4, the standard heuristic) is
            // marked as such, never presented as metered truth.
            "estimated_output_tokens": (assembled.text.chars().count() / 4).max(1),
            "token_estimate": "estimated", // honest marker: never metered truth
            "legs_detail": assembled.legs.iter().map(|l| serde_json::json!({
                "raced": l.raced, "debited": l.debited,
                "stop_used": l.stop_used, "debit": l.debit, "events": l.events,
                "chars": l.text.chars().count(),
            })).collect::<Vec<_>>(),
            "req_id": req_id,
            // Settles this chain reversed via /ai/v3/refund/ (0 on a clean run).
            "turns_refunded": refunded,
        })
    );
    ([("content-type", "text/event-stream")], event).into_response()
}

/// Real mode: assemble per-request deps mirroring native_complete, run the
/// engine over the live page stream, record health, answer SSE.
fn short_id(id: &AccountId) -> String {
    id.0.to_string()[..8.min(id.0.to_string().len())].to_string()
}

async fn custom_real(
    s: AppState,
    headers: &HeaderMap,
    body: &CompleteBody,
) -> Result<Response, ApiError> {
    // The prompt route wraps into ChainRequest; the dialect adapters
    // (ADR-003) build ChainRequest::Chat directly. One failover loop serves
    // both.
    let request = framer_custom::ChainRequest::Prompt(body.prompt.clone());
    custom_real_request(s, headers, request, &body.model, body.max_legs, body.stop).await
}

/// The account-failover loop over a ChainRequest: prompt route and dialect
/// adapters share selection, lease, health marking, and settle reversal.
async fn custom_real_request(
    s: AppState,
    headers: &HeaderMap,
    request: framer_custom::ChainRequest,
    model_str: &str,
    max_legs: Option<u32>,
    stop: Option<u32>,
) -> Result<Response, ApiError> {
    // ACCOUNT FAILOVER (2026-11-09): a Castle-refused open, a dead scope, or
    // lease/session/bearer trouble is ONE account's condition. The chain
    // used to run on a single picked account and return its error raw, and
    // nothing marked the account -- so the next request picked the SAME hot
    // account and hit the same wall (measured live: two consecutive custom
    // requests refused on e9d59f78 while eleven other accounts stood ready).
    // Account-local failures now retry on a fresh account (bounded at 3;
    // the attempt marks the custom-only refusal and excludes the failed
    // id). Request- or server-wide failures (gated model, wave pause,
    // saturated gate, a chain that already opened legs) return as-is --
    // retrying those buys nothing.
    let mut tried: Vec<String> = Vec::new();
    let mut last: Option<ApiError> = None;
    for _ in 0..3 {
        match custom_real_once(s.clone(), headers, request.clone(), model_str, max_legs, stop, &tried).await {
            Ok(resp) => return Ok(resp),
            Err((e, Some(failed))) => {
                eprintln!(
                    "[custom-route] account-local failure on {}; failing over to a fresh account",
                    &failed[..8.min(failed.len())],
                );
                tried.push(failed);
                last = Some(e);
            }
            Err((e, None)) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| {
        ApiError::ServingCapacity("custom route: no account could serve this request".into())
    }))
}

/// One account's attempt. The error tuple carries `Some(account_id)` when
/// the failure is local to that account (safe to fail over) and `None` when
/// it is request- or server-wide (return as-is).
async fn custom_real_once(
    s: AppState,
    headers: &HeaderMap,
    request: framer_custom::ChainRequest,
    model_str: &str,
    body_max_legs: Option<u32>,
    body_stop: Option<u32>,
    tried: &[String],
) -> Result<Response, (ApiError, Option<String>)> {
    // A caller-pinned account is strict: its failures never hop silently.
    let pinned = hdr(headers, "x-framerize-account").is_some();
    let local = |account: &framer_core::account::ManagedAccount| -> Option<String> {
        if pinned {
            None
        } else {
            Some(account.id.0.to_string())
        }
    };
    let req_id = uuid::Uuid::new_v4().to_string();
    let req_short = req_id[..8].to_string();
    if let Some(remaining) = s.wave.check() {
        return Err((
            ApiError::Cooling {
                retry_after_secs: remaining.as_secs().saturating_add(1),
            },
            None,
        ));
    }
    let pool = s
        .state
        .list()
        .await
        .map_err(|e| (ApiError::Internal(e), None))?;
    // Unpinned requests fail over across accounts (tried excludes the ones
    // this request already burned, and the custom-refusal marks skip
    // recently refused opens); a caller pin stays strict.
    let account = match hdr(headers, "x-framerize-account") {
        Some(pin) => s
            .serving_accounts
            .select(pool, Some(pin.as_str()))
            .map_err(|e| (e, None))?,
        None => s
            .serving_accounts
            .select_fresh(pool, tried)
            .map_err(|e| (e, None))?,
    };
    if open_refusal_active(&account.id.0.to_string()) {
        eprintln!(
            "[custom-route] req={req_short} account {} skipped (custom open refused it recently)",
            short_id(&account.id)
        );
        return Err((
            ApiError::ServingCapacity("custom open refused recently on this account".into()),
            local(&account),
        ));
    }
    // Same exclusion the free paths get: known-dead accounts (all workspaces
    // empty per the live ledger cache) never serve a leg.
    if !framer_core::rotation::account_might_bill(&account) {
        eprintln!(
            "[custom-route] req={req_short} account {} skipped (no funded workspace)",
            short_id(&account.id)
        );
        return Err((
            ApiError::ServingCapacity(
                "no funded workspace on the selected account; pick another or credit one".into(),
            ),
            local(&account),
        ));
    }
    eprintln!("[custom-route] req={req_short} stage=lease account={}", short_id(&account.id));
    let residency = RequestResidency::new(
        tokio::time::timeout(STAGE_TIMEOUT, lease_account(&s, account.id))
            .await
            .map_err(|_| {
                eprintln!("[custom-route] req={req_short} stage=lease timed out after 90s");
                (
                    ApiError::Internal("custom route: account lease timed out after 90s".into()),
                    local(&account),
                )
            })?
            .map_err(|e| (e, local(&account)))?,
    );
    let account = current_account(&s, account.id)
        .await
        .map_err(|e| (e, local(&account)))?;
    eprintln!("[custom-route] req={req_short} stage=session account={}", short_id(&account.id));
    let session = tokio::time::timeout(STAGE_TIMEOUT, s.sessions.load(&account.framer_user_id))
        .await
        .map_err(|_| {
            eprintln!("[custom-route] req={req_short} stage=session timed out after 90s");
            (
                ApiError::Internal("custom route: session load timed out after 90s".into()),
                local(&account),
            )
        })?
        .map_err(|e| (ApiError::Internal(e), local(&account)))?
        .ok_or_else(|| (ApiError::Internal("account has no session".into()), local(&account)))?;
    // Scope rule (matches the oracle the engine actually reads): the page
    // credits fetch hits scope_team, so the scope MUST be the account's live
    // current_team/current_project (the workspace the oracle + envelope bill
    // against) -- never a rotated extra workspace. pick_scope's rotation is
    // for the multi-workspace free chains; the custom route pins primary.
    let (project, scope_team) = match (
        account.current_team.clone(),
        account.current_project.clone(),
    ) {
        (Some(team), Some(proj)) => (proj, team),
        _ => {
            let scope = pick_scope(&account).map_err(|e| (e, local(&account)))?;
            (scope.project.clone(), scope.team.clone())
        }
    };
    let chat_session =
        hdr(headers, "x-framerize-session").unwrap_or_else(|| stable_session(&account.id));

    // Per (workspace x MODEL) concurrency, mirroring native_complete.
    // Model id rule (models.rs + live PROOF legs 21:26Z which streamed $0
    // with model=gpt-4.1): the /ai/v3/* gateway takes the UNPREFIXED form.
    // Strip any caller-supplied provider prefix at the boundary so the
    // envelope always carries the gateway form, whatever T3 sends.
    let model = {
        let m = model_str.trim();
        let m = if m.is_empty() { "gpt-4.1" } else { m };
        m.rsplit('/').next().unwrap_or(m).to_string()
    };
    // Direct-route models (gpt-6-sol & kin) exist only on /ai/v3/openai/
    // responses/ -- the OpenRouter passthroughs never carry them. The custom
    // route's cutoff makes them free the same way the chat route does: the
    // meter settles at response.completed, the page abort lands before it.
    // RED-ALERT GATE (2026-11-09): a raced direct-route leg BILLED -1.18 on
    // a live workspace (the terminal beat the abort). Fast models race often,
    // so the direct route is OFF by default: it stays opt-in behind
    // FRAMERIZE_CUSTOM_DIRECT=1 until the race is provably won.
    let dialect = if framer_core::convert::model_route(&model)
        == framer_core::convert::ModelRoute::DirectResponses
    {
        if std::env::var("FRAMERIZE_CUSTOM_DIRECT").map(|v| v == "1").unwrap_or(false) {
            framer_api::proxy::Dialect::OpenAiResponses
        } else {
            return Err((
                ApiError::Internal(format!(
                    "model {model} is a direct-route model; the custom route's abort can lose the race and bill (proved live: -1.18 on a raced leg). It is opt-in only (FRAMERIZE_CUSTOM_DIRECT=1)."
                )),
                None,
            ));
        }
    } else {
        DIALECT
    };
    let gate_model = framer_core::convert::to_openrouter_id(&model);
    let gate_key = format!("{}:{gate_model}", scope_team.0);
    // Bounded gate wait: native streams keepalives while queued, but custom
    // answers one buffered SSE frame -- an unbounded wait would be a silent
    // socket. 60s then a loud 503 (client retries; nothing spent, no leg
    // opened, lease still held and released on return).
    let _permit = tokio::time::timeout(std::time::Duration::from_secs(60), s.gate.acquire(&gate_key))
        .await
        .map_err(|_| {
            eprintln!("[custom-route] req={req_short} gate wait timed out (key={gate_key})");
            (
                ApiError::ServingCapacity(
                    "custom route concurrency gate is saturated; retry shortly".into(),
                ),
                None,
            )
        })?;
    current_account(&s, account.id)
        .await
        .map_err(|e| (e, local(&account)))?;

    // AI bearer JWT: the same per-account cached mint forward() uses.
    eprintln!("[custom-route] req={req_short} stage=bearer account={}", short_id(&account.id));
    let bearer = tokio::time::timeout(STAGE_TIMEOUT, s.client.custom_bearer(&session, &account.id))
        .await
        .map_err(|_| {
            eprintln!("[custom-route] req={req_short} stage=bearer timed out after 90s");
            (
                ApiError::Internal("custom route: bearer mint timed out after 90s".into()),
                local(&account),
            )
        })?
        .map_err(|e| (ApiError::Internal(e), local(&account)))?;
    // Seed the browser cookie jar (mirrors attempt: imported accounts start
    // with an empty jar; in-page fetches authenticate from it).
    eprintln!("[custom-route] req={req_short} stage=seed account={}", short_id(&account.id));
    tokio::time::timeout(
        STAGE_TIMEOUT,
        s.browser.seed_session(&account.id, &session.cookie_header),
    )
    .await
    .map_err(|_| {
        eprintln!("[custom-route] req={req_short} stage=seed timed out after 90s");
        (
            ApiError::Internal("custom route: session seed timed out after 90s".into()),
            local(&account),
        )
    })?
    .map_err(|e| (ApiError::Internal(e), local(&account)))?;

    // Direct-route (agent-panel) legs run under agents/message (the route's
    // own category); the chat passthrough keeps translation/translate.
    let category = if matches!(dialect, framer_api::proxy::Dialect::OpenAiResponses) {
        "agents".to_string()
    } else {
        std::env::var("FRAMERIZE_FEATURE_CATEGORY").unwrap_or_else(|_| "translation".to_string())
    };
    // Refund context: cloned BEFORE deps moves the originals, so a settled
    // chain can reverse its own turns after the engine returns.
    let refund_bearer = bearer.clone();
    let refund_project = project.clone();
    let refund_session = session.clone();
    let refund_step = chain_step(&category).to_string();
    let refund_category = category.clone();
    let opened_turns = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let team = scope_team.0.clone();
    let deps = RealDeps {
        account: account.id,
        session: session.clone(),
        project,
        bearer: bearer.clone(),
        category: category.clone(),
        step: chain_step(&category).to_string(),
        chat_session: chat_session.clone(),
        path: dialect.path(),
    };
    // No-spend preflight: the leased scope workspace must show a live
    // balance before any page opens. The daemon's cached CreditSnapshot is
    // advisory only (it went stale by 500 credits above); this GET costs
    // nothing and fails the request loud instead of burning an OPEN.
    {
        let live_credits = s
            .client
            .usage(&session, &scope_team)
            .await
            .map(|snap| snap.available)
            .unwrap_or(f64::NAN);
        eprintln!(
            "[custom-route] req={} stage=ledger account={} team={} live_credits={:.2}",
            req_short,
            &account.id.0.to_string()[..8.min(account.id.0.to_string().len())],
            &scope_team.0[..8.min(scope_team.0.len())],
            live_credits,
        );
        let floor = framer_core::rotation::bill_floor();
        if !live_credits.is_finite() || live_credits < floor {
            return Err((
                ApiError::Internal(format!(
                    "custom-route: scope workspace {} shows live balance {:.2} (need >= {:.0}); not opening a leg",
                    &scope_team.0[..8.min(scope_team.0.len())],
                    live_credits,
                    floor,
                )),
                local(&account),
            ));
        }
    }
    let page = framer_host::page_stream::HostPageStream::new(
        Arc::new(BridgePageEval {
            browser: s.browser.clone(),
        }),
        account.id,
        bearer,
        team.clone(),
        dialect.path().to_string(),
    );
    let mut settings = framer_custom::ChainSettings::default();
    if let Some(stop) = body_stop {
        settings.stop = framer_custom::clamped_stop(stop);
    }
    if let Some(max_legs) = body_max_legs {
        // Clamp at the route: an unbounded client value would tie one
        // account lease + gate permit for up to CHAIN_TIMEOUT (1h). The
        // engine hard-caps at 64; the route caps at the default 8 (a larger
        // run is a deliberate code change, not a request field).
        settings.max_legs = max_legs.min(8);
    }
    // Engine composition: BridgeStreamTransport (drain timeouts, stall
    // bounds, paced polls, stop plumbing) over the envelope-wrapping page
    // (model/stream injection + Gq envelope + 401 bearer refresh).
    let model_log = model.clone();
    let mut transport = framer_custom::BridgeStreamTransport::new(
        EnvelopePageStream {
            inner: page,
            deps,
            client: s.client.clone(),
            model,
            browser: s.browser.clone(),
            team,
            opened_turns: opened_turns.clone(),
        },
        if matches!(dialect, framer_api::proxy::Dialect::OpenAiResponses) {
            framer_custom::Dialect::Responses
        } else {
            framer_custom::Dialect::Chat
        },
    );
    eprintln!(
        "[custom-route] req={req_short} stage=open account={} model={model_log}",
        short_id(&account.id)
    );
    let result = tokio::time::timeout(
        ENGINE_TIMEOUT,
        framer_custom::complete_native(&mut transport, request, &settings),
    )
    .await
    .map_err(|_| {
        (
            ApiError::Internal(format!(
                "chain: engine run exceeded {}s with no terminal or stop (req {req_short})",
                ENGINE_TIMEOUT.as_secs()
            )),
            None,
        )
    })?;
    let result = residency.observe_result(account.id, result);
    record_account_health(&s, &account, &result).await;
    match result {
        Ok(mut assembled) => {
            s.gate.on_success(&gate_key).await;
            // SETTLE REVERSAL (2026-11-09): the abort keeps legs free, but an
            // early-stopped model's terminal can still settle (the runway
            // removed the SPEED race; a short complete answer is the
            // irreducible case) -- and concurrent traffic on the same
            // workspace can pollute the ledger delta. /ai/v3/refund/ has no
            // Castle layer, reverses a settled turn exactly, and is a denied
            // no-op on an unsettled one (verified 2026-10-05). So when the
            // chain's ledger moved, refund the turns this chain opened until
            // the balance recovers. Bounded: 4 posts, only on a real debit;
            // quota is ~2/window/account, so this stays rare-path only.
            let mut refunded = 0u32;
            if assembled.total_debit < -1e-6 {
                let before = s
                    .client
                    .usage(&refund_session, &scope_team)
                    .await
                    .map(|snap| snap.available)
                    .unwrap_or(f64::NAN);
                let turns: Vec<String> =
                    opened_turns.lock().map(|g| g.clone()).unwrap_or_default();
                for turn_id in turns.iter().take(4) {
                    let feature_name = if refund_category == "translation" {
                        "translation"
                    } else {
                        "agents"
                    };
                    let body = serde_json::json!({
                        "projectId": refund_project.0.as_str(),
                        "featureCategory": refund_category.as_str(),
                        "featureName": feature_name,
                        "step": refund_step.as_str(),
                        "sessionId": chat_session.as_str(),
                        "stepIndex": 0,
                        "turnId": turn_id.as_str(),
                        "userPrompt": "",
                        "castleToken": "garbage", // route ignores it (verified)
                    });
                    match s
                        .browser
                        .ai_post(&account.id, "/ai/v3/refund/", &refund_bearer, &body)
                        .await
                    {
                        Ok((200, _, text))
                            if text.contains("\"granted\":true")
                                || text.contains("\"granted\": true") =>
                        {
                            refunded += 1;
                            eprintln!(
                                "[custom-route] req={req_short} refund turn {} GRANTED (settle reversed)",
                                &turn_id[..8.min(turn_id.len())]
                            );
                        }
                        Ok((status, _, text)) => eprintln!(
                            "[custom-route] req={req_short} refund turn {} denied/HTTP {status}: {}",
                            &turn_id[..8.min(turn_id.len())],
                            &text[..text.len().min(120)]
                        ),
                        Err(e) => eprintln!(
                            "[custom-route] req={req_short} refund turn {} transport: {e}",
                            &turn_id[..8.min(turn_id.len())]
                        ),
                    }
                }
                if refunded > 0 {
                    let after = s
                        .client
                        .usage(&refund_session, &scope_team)
                        .await
                        .map(|snap| snap.available)
                        .unwrap_or(f64::NAN);
                    if before.is_finite() && after.is_finite() {
                        let recovered = after - before;
                        if recovered > 0.0 {
                            framer_core::rotation::note_live_balance(&scope_team.0, after);
                            assembled.total_debit += recovered;
                            eprintln!(
                                "[custom-route] req={req_short} refunds recovered {recovered:+.6}; adjusted total_debit={:+.6}",
                                assembled.total_debit
                            );
                        }
                    }
                }
            }
            eprintln!(
                "[custom-route] req={} account={} model={} legs={} chars={} total_debit={:+.6} refunded={} outcome={:?} session={}",
                req_short,
                &account.id.0.to_string()[..8.min(account.id.0.to_string().len())],
                model_log.clone(),
                assembled.legs.len(),
                assembled.text.len(),
                assembled.total_debit,
                refunded,
                assembled.outcome,
                chat_session,
            );
            let mut resp = sse_response(&assembled, &req_id, refunded);
            resp.headers_mut().insert(
                "x-framerize-session",
                chat_session
                    .parse()
                    .unwrap_or(axum::http::HeaderValue::from_static("")),
            );
            Ok(resp)
        }
        Err(e) => {
            eprintln!("[custom-route] req={} account={} model={} FAILED: {}",
                req_short,
                &account.id.0.to_string()[..8.min(account.id.0.to_string().len())],
                model_log,
                e.chars().take(300).collect::<String>());
            if e.contains("retryable 429") {
                s.gate.on_overloaded(&gate_key).await;
                if let Some(team) = account.current_team.as_ref() {
                    framer_core::rotation::mark_saturated(&team.0, 45_000);
                }
            }
            // A 402 means the leased scope workspace is truly out of credits
            // (provider-verified, not a guess). Mark the scope team saturated
            // for 5 minutes so later picks steer away from the dead team.
            if e.contains("402") || e.contains("credit_balance_exhausted")
                || e.contains("team_credit_balance_exhausted")
            {
                framer_core::rotation::mark_saturated(&scope_team.0, 300_000);
            }
            // An OPEN refusal (leg 1 never streamed) is this account's
            // custom-open pattern being risk-hot: mark it (custom-only --
            // the account keeps serving other routes) and fail over. A
            // failure after legs opened returns as-is.
            let retry = if e.contains("leg 1 failed") && e.contains("page stream open failed") {
                mark_open_refused(&account.id.0.to_string());
                local(&account)
            } else {
                None
            };
            Err((ApiError::Internal(e), retry))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_refusal_marks_are_custom_route_only_and_scoped() {
        let id = format!("test-acc-{}", uuid::Uuid::new_v4());
        assert!(!open_refusal_active(&id));
        mark_open_refused(&id);
        assert!(open_refusal_active(&id));
        // Other accounts are unaffected.
        assert!(!open_refusal_active(&format!("{id}-other")));
    }

    #[test]
    fn leg_cap_runway_guarantees_mid_generation_abort() {
        // Small stops keep the 4000 default.
        assert_eq!(leg_cap_with_runway(None, 100), 4_000);
        // A big stop_after raises the cap to 3x -- the server is still
        // generating when the abort fires at stop_after events.
        assert_eq!(leg_cap_with_runway(None, 2_000), 6_000);
        // A caller's small cap can never sit below the runway floor: that
        // was the bill-at-cap race (terminal beats the abort by design).
        assert_eq!(leg_cap_with_runway(Some(50), 200), 600);
        // Caller caps above the floor pass through.
        assert_eq!(leg_cap_with_runway(Some(8_000), 100), 8_000);
        // The worst-case settle stays bounded.
        assert_eq!(leg_cap_with_runway(None, 8_000), LEG_CAP_MAX);
        // Degenerate stops still get a sane cap.
        assert_eq!(leg_cap_with_runway(Some(1), 1), 64);
    }

    use framer_core::account::{AccountState, ManagedAccount};
    use framer_core::ids::{FramerUserId, ProjectId, TeamId};
    use framer_core::port::{CastleToken, ChainConfig, SessionStore, StateStore, WorkspaceSpec};
    use framer_core::workspace::CreditSnapshot;
    use framer_core::pipeline::{InferenceRequest, InferenceResult};
    use framer_core::convert::{AssembledResponse, AssembledResponses, ChainEvent};
    use futures::future::BoxFuture;

    /// Env-var guard: handler mode comes from the process env and tests run
    /// in parallel threads -- hold this across any test that sets/reads it.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn test_account() -> ManagedAccount {
        ManagedAccount {
            id: AccountId(uuid::Uuid::from_u128(7)),
            alias: "custom-fixture".into(),
            email: Some("custom-fixture@example.invalid".into()),
            framer_user_id: FramerUserId("user-7".into()),
            castle_device_id: "device-7".into(),
            proxy: None,
            current_team: Some(TeamId("team-7".into())),
            current_project: Some(ProjectId("project-7".into())),
            extra_workspaces: Vec::new(),
            state: AccountState::Ready,
            credit: None,
            consecutive_failures: 0,
            cooldown_step: 0,
            cooling_until_ms: None,
        }
    }

    /// Ports for custom-route tests: sessions/state/client answer from
    /// memory; the bridge answers eval_small from a script (canned page
    /// replies) and records seeded cookies. No browser, no network.
    struct CustomPorts {
        account: ManagedAccount,
        eval_script: std::sync::Mutex<std::collections::VecDeque<serde_json::Value>>,
        eval_seen: std::sync::Mutex<Vec<String>>,
        seeded: std::sync::Mutex<Vec<String>>,
    }

    impl CustomPorts {
        fn new(replies: Vec<serde_json::Value>) -> Self {
            Self {
                account: test_account(),
                eval_script: std::sync::Mutex::new(replies.into()),
                eval_seen: std::sync::Mutex::new(Vec::new()),
                seeded: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    macro_rules! denied {
        ($name:ident <$($lt:lifetime),*> ($($arg:ident: $ty:ty),*) -> $output:ty) => {
            fn $name<'s, $($lt,)* 'a>(&'s self, $($arg: $ty),*) -> BoxFuture<'a, $output>
            where 's: 'a, $($lt: 'a,)* Self: 'a {
                Box::pin(async move { panic!("unexpected custom fixture I/O: {}", stringify!($name)) })
            }
        };
    }

    impl BrowserBridge for CustomPorts {
        fn eval_small<'s, 'b, 'c, 'a>(
            &'s self,
            _account: &'b AccountId,
            expr: &'c str,
        ) -> BoxFuture<'a, Result<serde_json::Value, String>>
        where 's: 'a, 'b: 'a, 'c: 'a, Self: 'a {
            Box::pin(async move {
                self.eval_seen.lock().unwrap().push(expr.to_string());
                self.eval_script
                    .lock()
                    .unwrap()
                    .pop_front()
                    .ok_or_else(|| "custom fixture: eval script exhausted".to_string())
            })
        }

        fn seed_session<'s, 'b, 'c, 'a>(
            &'s self,
            _account: &'b AccountId,
            cookie: &'c str,
        ) -> BoxFuture<'a, Result<(), String>>
        where 's: 'a, 'b: 'a, 'c: 'a, Self: 'a {
            Box::pin(async move {
                self.seeded.lock().unwrap().push(cookie.to_string());
                Ok(())
            })
        }

        denied!(castle_token<'b>(_account: &'b AccountId) -> Result<CastleToken, String>);
        denied!(auth_begin<'b, 'c>(_account: &'b AccountId, _email: &'c str) -> Result<(), String>);
        denied!(auth_complete<'b, 'c>(_account: &'b AccountId, _token: &'c str) -> Result<(String, serde_json::Value), String>);
        denied!(ai_post<'b, 'c, 'd, 'e>(_account: &'b AccountId, _path: &'c str, _bearer: &'d str, _body: &'e serde_json::Value) -> Result<(u16, Option<String>, String), String>);
    }

    impl StateStore for CustomPorts {
        fn get<'s, 'a>(&'s self, id: AccountId) -> BoxFuture<'a, Result<Option<ManagedAccount>, String>>
        where 's: 'a, Self: 'a {
            Box::pin(async move {
                Ok((id == self.account.id).then(|| self.account.clone()))
            })
        }
        fn list<'s, 'a>(&'s self) -> BoxFuture<'a, Result<Vec<ManagedAccount>, String>>
        where 's: 'a, Self: 'a {
            Box::pin(async move { Ok(vec![self.account.clone()]) })
        }
        fn upsert<'s, 'b, 'a>(&'s self, account: &'b ManagedAccount) -> BoxFuture<'a, Result<(), String>>
        where 's: 'a, 'b: 'a, Self: 'a {
            Box::pin(async move {
                assert_eq!(account.id, self.account.id);
                Ok(())
            })
        }
        denied!(remove<>(_id: AccountId) -> Result<(), String>);
    }

    impl SessionStore for CustomPorts {
        fn load<'s, 'b, 'a>(
            &'s self,
            user: &'b FramerUserId,
        ) -> BoxFuture<'a, Result<Option<Session>, String>>
        where 's: 'a, 'b: 'a, Self: 'a {
            Box::pin(async move {
                Ok(Some(Session {
                    active_user: user.clone(),
                    cookie_header: "session=custom-fixture".into(),
                }))
            })
        }
        denied!(save<'b>(_session: &'b Session) -> Result<(), String>);
        denied!(delete<'b>(_user: &'b FramerUserId) -> Result<(), String>);
    }

    impl framer_core::port::FramerClient for CustomPorts {
        fn custom_bearer<'s, 'b, 'c, 'a>(
            &'s self,
            _session: &'b Session,
            _account: &'c AccountId,
        ) -> BoxFuture<'a, Result<String, String>>
        where 's: 'a, 'b: 'a, 'c: 'a, Self: 'a {
            Box::pin(async move { Ok("custom-test-bearer".into()) })
        }
        denied!(request_magic_link<'b, 'c>(_email: &'b str, _castle: &'c CastleToken) -> Result<(), String>);
        denied!(complete_magic_link<'b, 'c>(_token: &'b str, _castle: &'c CastleToken) -> Result<Session, String>);
        denied!(me<'b>(_session: &'b Session) -> Result<serde_json::Value, String>);
        denied!(run_inference<'b, 'c, 'd, 'e, 'f>(_session: &'b Session, _browser: &'c dyn BrowserBridge, _account: &'d AccountId, _project: &'e ProjectId, _req: &'f InferenceRequest) -> Result<InferenceResult, String>);
        denied!(proxy<'b, 'c, 'd, 'e, 'f, 'g>(_session: &'b Session, _browser: &'c dyn BrowserBridge, _account: &'d AccountId, _project: &'e ProjectId, _dialect: &'f str, _payload: serde_json::Value, _chat: &'g str) -> Result<framer_core::port::ProxyResponse, String>);
        denied!(freechat_long<'b, 'c, 'd, 'e, 'f, 'g, 'h>(_session: &'b Session, _browser: &'c dyn BrowserBridge, _account: &'d AccountId, _project: &'e ProjectId, _payload: serde_json::Value, _cfg: &'f ChainConfig, _chat: &'g str, _events: Option<&'h (dyn Fn(ChainEvent) + Send + Sync)>) -> Result<AssembledResponse, String>);
        denied!(native_long<'b, 'c, 'd, 'e, 'f, 'g, 'h>(_session: &'b Session, _browser: &'c dyn BrowserBridge, _account: &'d AccountId, _project: &'e ProjectId, _payload: serde_json::Value, _cfg: &'f ChainConfig, _chat: &'g str, _events: Option<&'h (dyn Fn(ChainEvent) + Send + Sync)>) -> Result<AssembledResponses, String>);
        fn usage<'s, 'b, 'c, 'a>(
            &'s self,
            _session: &'b Session,
            _team: &'c TeamId,
        ) -> BoxFuture<'a, Result<CreditSnapshot, String>>
        where 's: 'a, 'b: 'a, 'c: 'a, Self: 'a {
            // Fixture live balance: comfortably above the preflight floor so
            // the no-spend gate passes and legs run.
            Box::pin(async move {
                Ok(CreditSnapshot { available: 500.0, funds: 1, grants: 1, allocated: 500.0 , renews_at: None})
            })
        }
        denied!(claim_workspace_grant<'b>(_session: &'b Session) -> Result<bool, String>);
        denied!(create_project<'b, 'c>(_session: &'b Session, _team: &'c TeamId) -> Result<ProjectId, String>);
        denied!(team_projects<'b, 'c>(_session: &'b Session, _team: &'c TeamId) -> Result<Vec<ProjectId>, String>);
        denied!(create_workspace<'b, 'c>(_session: &'b Session, _spec: &'c WorkspaceSpec) -> Result<TeamId, String>);
    }

    fn test_state_with(replies: Vec<serde_json::Value>) -> (AppState, Arc<CustomPorts>) {
        let ports = Arc::new(CustomPorts::new(replies));
        let state = AppState {
            client: ports.clone(),
            browser: ports.clone(),
            sessions: ports.clone(),
            state: ports.clone(),
            gate: Arc::new(framer_core::gate::ConcurrencyGate::new()),
            wave: Arc::new(framer_core::wave::WaveGuard::new()),
            serving_accounts: Arc::new(
                super::super::ServingAccountPolicy::from_config(
                    super::super::PoolConfig::default(),
                )
                .unwrap(),
            ),
        };
        (state, ports)
    }

    #[tokio::test]
    async fn mock_complete_serves_assembled_sse() {
        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var(TRANSPORT_ENV, "mock");
        let app = Router::new().nest("/api/custom", custom_routes(test_state_with(Vec::new()).0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let payload =
            serde_json::json!({"prompt": "explain X", "model": "gpt-4.1"}).to_string();
        let req = format!(
            "POST /api/custom/complete HTTP/1.1\r\nhost: 127.0.0.1\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            payload.len(),
            payload
        );
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw);
        let head = &text[..text.len().min(200)];
        assert!(text.contains("200"), "status line: {head}");
        assert!(text.contains("text/event-stream"), "sse content type");
        assert!(text.contains("data: "), "sse frame");
        assert!(text.contains("first prefix here"), "assembled text");
    }

    /// End-to-end real mode with a scripted page: every leg races
    /// (terminal beats the stop) with no ledger movement, so all 8 legs
    /// accept and the chain ends LegCap. Script per leg: before-credits,
    /// OPEN ok, DRAIN raced text, after-credits. Zero browser, zero network.
    #[tokio::test]
    async fn real_mode_runs_legs_through_envelope_with_bearer() {
        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var(TRANSPORT_ENV, "real");
        // Leg 1: two text slices (each its own drain batch) then a raced
        // terminal. Captured "alpha beta " (kept: trim vs empty anchor is a
        // no-op, growth 11 chars < 40 -> near-zero FINISH leg... so instead
        // make leg 1 long: 3 slices of 20 chars = 60 chars of growth, kept
        // and chained; leg 2 (continuation) races immediately with short
        // text -> near-zero -> Capped finish. Two legs, deterministic.
        let long = "abcdefghijklmnopqrst "; // 21 chars per slice
        let script = vec![
            serde_json::json!({"status": 200, "available": 100.0}),
            serde_json::json!({"ok": true, "status": 200}),
            serde_json::json!({"present": true, "text": long, "done": false, "stopped": false, "error": ""}),
            serde_json::json!({"present": true, "text": long, "done": false, "stopped": false, "error": ""}),
            serde_json::json!({"present": true, "text": long, "done": false, "stopped": false, "error": ""}),
            serde_json::json!({"present": true, "text": "", "done": true, "stopped": false, "error": ""}),
            serde_json::json!({"status": 200, "available": 100.0}),
            serde_json::json!({"status": 200, "available": 100.0}),
            serde_json::json!({"ok": true, "status": 200}),
            serde_json::json!({"present": true, "text": "tail ", "done": true, "stopped": false, "error": ""}),
            serde_json::json!({"status": 200, "available": 100.0}),
        ];
        // NOTE: open() now pre-cancels the slot (stale-slot recovery) and
        // the page-pin path verifies the fresh slot with one DRAIN
        // (present/absent check) before the engine drains: each OPEN costs
        // one CANCEL reply + one verify-DRAIN reply. The script inserts one
        // of each right after each before-credits read (credits, pre-cancel,
        // open, verify-drain, ...). The verify drain reports the fresh slot
        // (present, empty, not done) so open_stream proceeds to the engine.
        let script = {
            let mut s = script;
            s.insert(1, serde_json::json!({"cancelled": false, "reason": "no-stream"}));
            s.insert(3, serde_json::json!({"present": true, "text": "", "done": false, "stopped": false, "status": 200, "error": ""}));
            s.insert(10, serde_json::json!({"cancelled": false, "reason": "no-stream"}));
            s.insert(12, serde_json::json!({"present": true, "text": "", "done": false, "stopped": false, "status": 200, "error": ""}));
            s
        };
        let (state, ports) = test_state_with(script);
        let resp = custom_complete_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(CompleteBody { prompt: "explain X".into(), model: "".into(), max_legs: None, stop: None }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 256 * 1024).await.unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("data: "), "sse frame: {text}");
        assert!(text.contains("abcdefghijklmnopqrst"), "assembled leg text: {text}");
        assert!(text.contains("legs"), "legs recorded: {text}");
        // The OPEN carried the full Gq envelope, not the bare leg.
        // (First eval is the before-leg credits read; find the fetch open.)
        let seen = ports.eval_seen.lock().unwrap();
        let open = seen
            .iter()
            .find(|e| e.contains("/ai/v3/chat/"))
            .expect("page fetch was opened");
        assert!(open.contains("project-7"), "envelope projectId: {open}");
        assert!(open.contains("custom-test-bearer"), "fetch bearer: {open}");
        assert!(open.contains("/ai/v3/chat/"), "chat route: {open}");
        assert!(open.contains("translation"), "category: {open}");
        assert!(open.contains("gpt-4.1"), "gateway model: {open}");
        assert!(!open.contains("openai/gpt-4.1"), "gateway form unprefixed: {open}");
        assert!(open.contains("explain X"), "leg prompt inside request: {open}");
        assert!(open.contains("request"), "native payload nested under request: {open}");
        assert!(open.contains(", 200)"), "rung stop baked into page open: {open}");
        drop(seen);
        // Session jar seeded exactly once (mirrors attempt).
        let seeded = ports.seeded.lock().unwrap();
        assert_eq!(seeded.as_slice(), ["session=custom-fixture"]);
    }

    #[tokio::test]
    async fn real_mode_rejects_empty_prompt_before_io() {
        let (state, ports) = test_state_with(Vec::new());
        let resp = custom_complete_handler(
            State(state),
            HeaderMap::new(),
            Json(CompleteBody { prompt: "  ".into(), model: "".into(), max_legs: None, stop: None }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(ports.eval_seen.lock().unwrap().is_empty(), "no page eval on bad request");
    }

    #[tokio::test]
    async fn real_mode_unknown_account_fails_closed() {
        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var(TRANSPORT_ENV, "real");
        let (state, ports) = test_state_with(Vec::new());
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-framerize-account",
            "00000000-0000-0000-0000-000000000000".parse().unwrap(),
        );
        let resp = custom_complete_handler(
            State(state),
            headers,
            Json(CompleteBody { prompt: "explain X".into(), model: "".into(), max_legs: None, stop: None }),
        )
        .await;
        // Unknown account pin: no eligible account, no page touched.
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(ports.eval_seen.lock().unwrap().is_empty(), "no page eval without admission");
    }
}
