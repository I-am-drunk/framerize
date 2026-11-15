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

use crate::{retain_owner, AbortOnDrop};
use super::{current_account, hdr, pick_funded_scope, record_account_health, stable_session, ApiError, AppState};
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
        .route("/complete/status", axum::routing::get(status_handler).post(status_reset_handler))
        .route("/refund_lab", axum::routing::post(refund_lab_handler))
        .route("/sweep_probe", axum::routing::post(sweep_probe_handler))
        .route("/claim_grant", axum::routing::post(claim_grant_handler))
        .with_state(state)
}

async fn sweep_probe_handler(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let acct = headers.get("x-framerize-account").and_then(|v| v.to_str().ok()).map(|x| x.to_string());
    let sel = crate::select_funded_account(&s, acct).await;
    let (account, _res, session, scope) = match sel {
        Ok(t) => t,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": format!("no funded: {e:?}")}))).into_response(),
    };
    let bearer = match s.client.custom_bearer(&session, &account.id).await {
        Ok(b) => b,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": format!("bearer: {e}")}))).into_response(),
    };
    if s.browser.seed_session(&account.id, &session.cookie_header).await.is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": "seed fail"}))).into_response()
    };
    // Fully-flexible billing probe: control the upstream `path` and the raw native
    // `payload` (or build from model/prompt/max_tokens). `stream_mode` controls how the
    // stream key is sent: "false" (explicit), "true", "omit" (key absent), "null", "string".
    // Measures the real ledger delta across settle_secs. Optional refund (do_refund).
    let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("/ai/v3/chat/").to_string();
    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("anthropic/claude-fable-5.2").to_string();
    let stream_mode = body.get("stream_mode").and_then(|v| v.as_str()).unwrap_or("false").to_string();
    let max_tokens = body.get("max_tokens").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    let do_refund = body.get("do_refund").and_then(|v| v.as_bool()).unwrap_or(false);
    let settle_secs = body.get("settle_secs").and_then(|v| v.as_u64()).unwrap_or(75);
    let prompt = body.get("prompt").and_then(|v| v.as_str()).unwrap_or("Say ok").to_string();
    let n = body.get("n").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let custom_payload = body.get("payload").cloned();
    let feature_category = body.get("feature_category").and_then(|v| v.as_str()).unwrap_or("translation").to_string();
    let feature_name = body.get("feature_name").and_then(|v| v.as_str()).unwrap_or("translation").to_string();
    let step = body.get("step").and_then(|v| v.as_str()).unwrap_or("translate").to_string();
    let led0 = s.client.usage(&session, &scope.team).await.map(|x| x.available).unwrap_or(f64::NAN);
    let mut results: Vec<serde_json::Value> = Vec::new();
    for i in 0..n {
        let mut payload = custom_payload.clone().unwrap_or_else(|| serde_json::json!({"model": model, "messages": [{"role": "user", "content": prompt}], "max_tokens": max_tokens}));
        match stream_mode.as_str() {
            "true" => { payload["stream"] = serde_json::json!(true); }
            "omit" => { payload.as_object_mut().map(|o| o.remove("stream")); }
            "null" => { payload["stream"] = serde_json::Value::Null; }
            "string" => { payload["stream"] = serde_json::json!("false"); }
            _ => { payload["stream"] = serde_json::json!(false); }
        }
        let sess = format!("probe-{}-{}", i, &uuid::Uuid::new_v4().simple().to_string()[..6]);
        let envelope = if let Some(tid) = body.get("turn_id").and_then(|v| v.as_str()) {
            framer_api::proxy::envelope_raw4(&scope.project, &feature_category, &feature_name, &step, &model, &payload, &sess, 0, Some(tid))
        } else {
            framer_api::proxy::envelope_raw3(&scope.project, &feature_category, &feature_name, &step, &model, &payload, &sess, 0)
        };
        let turn = envelope["turnId"].as_str().unwrap_or("").to_string();
        let serve = s.browser.ai_post(&account.id, &path, &bearer, &envelope).await;
        let (st, uh, txt) = match serve {
            Ok(t) => t,
            Err(e) => { results.push(serde_json::json!({"i": i, "serve_ok": false, "err": e})); continue; }
        };
        let snippet: String = txt.chars().take(4000).collect();
        let mut row = serde_json::json!({"i": i, "serve_status": st, "usage_hdr": uh, "turn": &turn[..8.min(turn.len())], "turn_full": turn, "body": snippet});
        if do_refund {
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
            let rbody = serde_json::json!({"projectId": scope.project.0.as_str(), "featureCategory": feature_category.as_str(), "featureName": feature_name.as_str(), "step": step.as_str(), "sessionId": sess.as_str(), "stepIndex": 0, "turnId": turn.as_str(), "userPrompt": "", "castleToken": "garbage"});
            match s.browser.ai_post(&account.id, "/ai/v3/refund/", &bearer, &rbody).await {
                Ok((rst, _, rtxt)) => {
                    let granted = rtxt.contains("granted") && rtxt.contains("true");
                    row["granted"] = serde_json::json!(granted);
                    row["refund_status"] = serde_json::json!(rst);
                }
                Err(e) => { row["refund_err"] = serde_json::json!(e); }
            }
        }
        results.push(row);
    }
    tokio::time::sleep(std::time::Duration::from_secs(settle_secs)).await;
    let led1 = s.client.usage(&session, &scope.team).await.map(|x| x.available).unwrap_or(f64::NAN);
    Json(serde_json::json!({
        "account": &account.id.0.to_string()[..8], "team": &scope.team.0[..8], "model": model, "path": path, "stream_mode": stream_mode, "n": n,
        "results": results, "ledger_before": led0, "ledger_after": led1, "final_delta": led1 - led0,
    })).into_response()
}


/// RESEARCH: claim_workspace_grant on an existing account. Framer grants 500
/// free-intro credits to the first TWO workspaces on an account. Accounts imported
/// with a pinned team_id skipped the claim, so they hold one grant. This creates the
/// second workspace and measures whether the second 500-credit grant lands.
async fn claim_grant_handler(
    State(s): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let acct = body.get("account_id").and_then(|v| v.as_str()).map(|x| x.to_string());
    let sel = crate::select_funded_account(&s, acct).await;
    let (account, _res, session, scope) = match sel {
        Ok(t) => t,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": format!("no funded: {e:?}")}))).into_response(),
    };
    let total_credits = |s: AppState, session: framer_core::port::Session| async move {
        let me = match s.client.me(&session).await { Ok(m) => m, Err(_) => return f64::NAN };
        let orgs: Vec<&serde_json::Value> = me["spaces"].as_array()
            .map(|a| a.iter().filter(|sp| sp["scope"].as_str() == Some("organization")).collect())
            .unwrap_or_default();
        let mut total = 0.0;
        for sp in orgs {
            if let Some(tid) = sp["ownerId"].as_str() {
                let team = framer_core::ids::TeamId(tid.to_string());
                if let Ok(snap) = s.client.usage(&session, &team).await { total += snap.available; }
            }
        }
        total
    };
    let before = total_credits(s.clone(), session.clone()).await;
    let claimed = s.client.claim_workspace_grant(&session).await;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let after = total_credits(s.clone(), session.clone()).await;
    Json(serde_json::json!({
        "account": &account.id.0.to_string()[..8],
        "team": &scope.team.0[..8.min(scope.team.0.len())],
        "claimed": format!("{:?}", claimed),
        "credits_before": before,
        "credits_after": after,
        "delta": after - before,
    })).into_response()
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
/// Also the spend-lock dashboard: allowlist, breaker state. Operators reset
/// a latched breaker with POST {"reset_spend_breaker":true} -- only after
/// re-proving the meter is back to terminal-frame settle.
async fn status_handler() -> Response {
    Json(serde_json::json!({
        "custom_route": "ok",
        "spend_lock": {
            "allowlist": ["gpt-4.1-mini", "claude-haiku-4.5", "gpt-6-astra"],
            "breaker_tripped": framer_custom::ledger::spend_breaker_tripped(),
        },
    }))
    .into_response()
}

/// REFUND LAB (FRAMERIZE_REFUND_LAB=1 only; default: loud refusal, zero surface):
/// refund ONE arbitrary settled turn with overrideable tuple fields, to reverse-engineer
/// quota/idempotency semantics. Every post is unmetered by route design; the lab changes
/// nothing about what the server counts.
async fn refund_lab_handler(
    State(s): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if std::env::var("FRAMERIZE_REFUND_LAB").map(|v| v == "1").unwrap_or(false) == false {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "refund lab disabled (FRAMERIZE_REFUND_LAB=1 to enable)"})),
        )
            .into_response();
    }
    let turn_id = body.get("turn_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if turn_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "turn_id required"})),
        )
            .into_response();
    }
    // Reuse the funded-account path (same account the turn settled on, by operator choice).
    let account_hdr = body.get("account").and_then(|v| v.as_str()).map(|x| x.to_string());
    let sel = crate::select_funded_account(&s, account_hdr).await;
    let (account, _residency, session, scope) = match sel {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": format!("no funded account: {e:?}")})),
            )
                .into_response();
        }
    };
    let fresh = body.get("fresh").and_then(|v| v.as_bool()).unwrap_or(false);
    let bearer_res = if fresh {
        s.client.custom_bearer_fresh(&session, &account.id).await
    } else {
        s.client.custom_bearer(&session, &account.id).await
    };
    let bearer = match bearer_res {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": format!("bearer mint failed: {e}")})),
            )
                .into_response();
        }
    };
    if s.browser.seed_session(&account.id, &session.cookie_header).await.is_err() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "session seed failed"})),
        )
            .into_response();
    }
    let project = scope.project.clone();
    let category = body.get("category").and_then(|v| v.as_str()).unwrap_or("translation").to_string();
    let feature = body.get("feature").and_then(|v| v.as_str()).unwrap_or("translation").to_string();
    let fname = body.get("feature_name").and_then(|v| v.as_str()).unwrap_or(&feature).to_string();
    let step = body.get("step").and_then(|v| v.as_str()).map(|x| x.to_string()).unwrap_or_else(|| chain_step(&category).to_string());
    let session_id = body.get("session_id").and_then(|v| v.as_str()).unwrap_or("lab").to_string();
    let step_index = body.get("step_index").and_then(|v| v.as_u64()).unwrap_or(0);
    let rbody = serde_json::json!({
        "projectId": project.0.as_str(),
        "featureCategory": category.as_str(),
        "featureName": fname.as_str(),
        "step": step.as_str(),
        "sessionId": session_id.as_str(),
        "stepIndex": step_index,
        "turnId": turn_id.as_str(),
        "userPrompt": "",
        "castleToken": "garbage",
    });
    match s.browser.ai_post(&account.id, "/ai/v3/refund/", &bearer, &rbody).await {
        Ok((status, _, text)) => {
            let granted = text.contains("\"granted\":true") || text.contains("\"granted\": true");
            eprintln!("[refund-lab] turn {} session={} step={} feature={} -> HTTP {} granted={} body={}",
                &turn_id[..8.min(turn_id.len())], session_id, step, fname, status, granted,
                &text[..text.len().min(200)]);
            Json(serde_json::json!({
                "turn": &turn_id[..8.min(turn_id.len())],
                "status": status,
                "granted": granted,
                "session_id": session_id,
                "step": step,
                "body": &text[..text.len().min(500)],
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": format!("transport: {e}")})),
        )
            .into_response(),
    }
}

async fn status_reset_handler(Json(body): Json<serde_json::Value>) -> Response {
    if body.get("reset_spend_breaker").and_then(|v| v.as_bool()) == Some(true) {
        framer_custom::ledger::reset_spend_breaker();
        return Json(serde_json::json!({"spend_breaker": "reset"})).into_response();
    }
    Json(serde_json::json!({
        "custom_route": "ok",
        "spend_lock": {
            "allowlist": ["gpt-4.1-mini", "claude-haiku-4.5", "gpt-6-astra"],
            "breaker_tripped": framer_custom::ledger::spend_breaker_tripped(),
        },
    }))
    .into_response()
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
        // Model id rule (2026-11-11 -- the bare-id bug): the envelope must carry the
        // OpenRouter CATALOG form (anthropic/claude-haiku-4.5, moonshotai/kimi-k3,
        // openai/gpt-4.1). Bare ids 400 ("not a valid model ID") or route to a different
        // meter than the freechat legs that settle $0. convert::to_openrouter_id is the
        // single mapping (same one the native paths use).
        let model = framer_core::convert::to_openrouter_id(&self.model);
        native["model"] = serde_json::Value::String(model);
        // Token cap name per route: the direct Responses route takes
        // max_output_tokens (and 400s "Unknown parameter: 'max_tokens'");
        // the chat route takes max_tokens. Either way the cap obeys the
        // RUNWAY RULE (leg_cap_with_runway): never below 3x stop_after, so a
        // fast model cannot reach the terminal before the in-page abort.
        // OPENROUTER-ROUTED MODELS (2026-11-10, live-proved): the envelope
        // model id is the gateway form (e.g. kimi-k3), which the gateway
        // forwards to OpenRouter (wire tap: model "moonshotai/kimi-k3"):
        // OpenRouter bills per generated token as they stream, so a huge
        // runway cap lets the leg run hundreds of deltas before the stop
        // and the meter settles large (measured: -2.67 on a 193-event leg
        // at cap 4000). For these models the cap IS the spend bound: pin
        // it tight (stop_after floor 64) so the stream can only ever cost
        // the floor even if the abort lands late. OpenAI-routed models
        // (gpt-4.1-mini et al) settle only on the terminal frame, so they
        // keep the full runway.
        let openrouter_routed = self.model.starts_with("kimi")
            || self.model.starts_with("deepseek")
            || self.model.starts_with("grok")
            || self.model.starts_with("glm")
            || self.model.starts_with("muse");
        let cap_key = if self.deps.path.contains("responses") {
            "max_output_tokens"
        } else {
            "max_tokens"
        };
        let caller_cap = native.get(cap_key).and_then(|v| v.as_u64());
        let cap = if openrouter_routed {
            // Tight cap: the meter prices generated tokens, so bound the
            // worst case to the floor regardless of abort timing. The leg
            // still carries the full prompt + padding instruction; the cap
            // only truncates generation length, never the prompt.
            caller_cap.unwrap_or(16).max(8).min(32)
        } else {
            leg_cap_with_runway(caller_cap, stop_after)
        };
        native[cap_key] = serde_json::Value::Number(serde_json::Number::from(cap));
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
        // SPEND GATE at the page-stream mouth (the OPEN path bypasses both
        // bridge ai_post impls and fires the in-page fetch directly): refuse
        // before wrap/eval when the model is not zero-proved.
        if !framer_core::rotation::spend_allowed(&self.model) {
            let reason = framer_core::rotation::spend_denied_reason(&self.model);
            eprintln!("[spend-gate] STREAM DENIED model={} -- {reason}", self.model);
            return Err(reason);
        }
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

/// REFUND SWEEPER: unrecovered debits persist to a JSONL journal; every later refund
/// cycle also retries aged entries (refund posts are unmetered, so retries are free).
/// Grants arrive in bursts (~2/window); the sweeper captures the next burst automatically
/// instead of losing denied turns forever.
fn sweeper_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("FRAMERIZE_REFUND_JOURNAL") {
        if !p.is_empty() {
            return Some(std::path::PathBuf::from(p));
        }
    }
    std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join(".framerize/refund_queue.jsonl"))
}
fn sweeper_append(account: &str, turn_id: &str, debit: f64) {
    let Some(path) = sweeper_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let line = serde_json::json!({"ts": framer_core::rotation::now_ms(), "account": account, "turn_id": turn_id, "debit": debit}).to_string();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write;
        let _ = writeln!(f, "{line}");
    }
}
fn sweeper_take(max_age_ms: u64) -> Vec<(String, String, f64)> {
    // Read + keep entries younger than max_age; drop the rest. Returns (account, turn, debit).
    let Some(path) = sweeper_path() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
    let now = framer_core::rotation::now_ms();
    let mut keep = Vec::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let age = now.saturating_sub(v.get("ts").and_then(|x| x.as_u64()).unwrap_or(0));
        let acc = v.get("account").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let turn = v.get("turn_id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let debit = v.get("debit").and_then(|x| x.as_f64()).unwrap_or(0.0);
        if age > max_age_ms || acc.is_empty() || turn.is_empty() {
            continue;
        }
        keep.push(line.to_string());
        out.push((acc, turn, debit));
    }
    let _ = std::fs::write(&path, keep.join("\n") + if keep.is_empty() { "" } else { "\n" });
    out
}
fn sweeper_drop(account: &str, turn_id: &str) {
    let Some(path) = sweeper_path() else { return };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { return true };
            !(v.get("account").and_then(|x| x.as_str()) == Some(account)
                && v.get("turn_id").and_then(|x| x.as_str()) == Some(turn_id))
        })
        .collect();
    let _ = std::fs::write(&path, kept.join("\n") + if kept.is_empty() { "" } else { "\n" });
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

/// What a successful chain run produced -- the parts every renderer needs.
/// The prompt route renders the custom SSE frame; the dialect adapters
/// (ADR-003) render their own shapes from the same parts.
pub(super) struct CutoffOutcome {
    pub assembled: framer_custom::Assembled,
    pub req_id: String,
    pub chat_session: String,
    pub refunded: u32,
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
    let out = cutoff_run(s, headers, request, model_str, max_legs, stop).await?;
    // QUARANTINED legs billed: never serve billed text as a clean 200. The
    // frame carries the honest debit + Quarantined outcome with empty text
    // so no caller can mistake a spent leg for a free answer (2026-11-10:
    // per-token meters settle every delta; retries re-bill; the only safe
    // shape is fail-loud with books kept).
    if matches!(
        out.assembled.outcome,
        framer_custom::ChainOutcome::Quarantined
    ) {
        return Ok((
            axum::http::StatusCode::PAYMENT_REQUIRED,
            axum::Json(serde_json::json!({
                "error": "custom route: leg settled nonzero and was quarantined (no free path this attempt)",
                "outcome": format!("{:?}", out.assembled.outcome),
                "total_debit": out.assembled.total_debit,
                "req_id": out.req_id,
            })),
        )
            .into_response());
    }
    let mut resp = sse_response(&out.assembled, &out.req_id, out.refunded);
    resp.headers_mut().insert(
        "x-framerize-session",
        out.chat_session
            .parse()
            .unwrap_or(axum::http::HeaderValue::from_static("")),
    );
    Ok(resp)
}

/// Run the cutoff chain with account failover; return the raw parts for
/// whatever renderer the caller speaks (custom SSE frame or a dialect).
/// SPEND LOCK (2026-11-10): the ONLY models the cutoff chain may serve.
/// A model is listed here iff its upstream meter settles EXCLUSIVELY on the
/// terminal/usage frame, so the in-page abort (which lands before the
/// terminal by construction) can never be billed -- live-proved over 100+
/// legs each. Every other model id -> loud InvalidRequest BEFORE any
/// account lease, browser page, or upstream open, so accidental spend is
/// structurally impossible on this path (not a matter of abort timing).
/// Non-exhaustive DENY rationale (each proved live): kimi/deepseek/grok/glm/
/// muse/gemini ride OpenRouter per-token settle (every delta bills, floor
/// -0.05 on a 1-event abort); gpt-4.1 full-size settles short bursts (-0.08
/// exact); direct-route sol kin race the terminal (-1.18).
pub fn cutoff_free_model(model: &str) -> bool {
    // Single source of truth: model::free_on_cutoff (zero-proved BASE ids only).
    framer_core::model::free_on_cutoff(model)
}

/// Loud refusal for a non-allowlisted model on the cutoff path.
fn cutoff_model_denied(model: &str) -> ApiError {
    ApiError::InvalidRequest(format!(
        "model {model} is not on the zero-spend allowlist (cutoff chain serves ONLY gpt-4.1-mini + claude-haiku-4.5 + gpt-6-astra + claude-fable-5.2 (case-bypassed past the workspace plan-gate, proved $0); every other upstream meter bills per-token or per-burst -- proved live). No account was leased, no page opened, nothing spent."
    ))
}


pub(super) async fn cutoff_run(
    s: AppState,
    headers: &HeaderMap,
    request: framer_custom::ChainRequest,
    model_str: &str,
    max_legs: Option<u32>,
    stop: Option<u32>,
) -> Result<CutoffOutcome, ApiError> {
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
    // SPEND LOCK: model gate FIRST -- before pool reads, leases, pages.
    // No allowlisted model => no upstream open exists downstream, so spend
    // is impossible regardless of abort timing, retries, or fallbacks.
    // NITRO-MAP (single source = model::effective_upstream_model): bare premium ids
    // map to their :nitro/:floor endpoints BEFORE gating, so the allowlist sees the
    // mapped id (the cutoff path aborts mid-stream; nitro meters settle $0 regardless).
    let mapped_model = framer_core::model::effective_upstream_model(model_str);
    let model_str: &str = if mapped_model != model_str {
        // mapped owns the String -- leak is wrong; instead re-enter via owned:
        // (see below: custom_real_once takes &str; we pass &mapped_model)
        &mapped_model
    } else {
        model_str
    };
    // PROVIDER-CASE NORMALIZATION (2026-11-15, live-proved): an UPPERCASE
    // provider prefix (ANTHROPIC/x) routes upstream to a METERED provider
    // endpoint (50% billed, -$149 over 52 legs) while the lowercase form
    // rides the grant-covered native endpoint (~1% micro-quanta). The gate
    // below slug-matches either form, so caps legs OPEN — then bill. Rewrite
    // to lowercase-provider BEFORE gating so every leg opens on the free
    // endpoint. Same upstream model (echoes identical), strictly dominant.
    // Owned String so the borrow lives past this block.
    let model_lowered: String;
    let model_str: &str = {
        let t = model_str.trim();
        match t.split_once('/') {
            Some((p, rest)) if p.chars().any(|c| c.is_ascii_uppercase()) => {
                model_lowered = format!("{}/{}", p.to_ascii_lowercase(), rest);
                &model_lowered
            }
            _ => model_str,
        }
    };
    // NITRO-STRIP FOR ALLOWLISTED BASES (2026-11-15): :nitro on a free base
    // buys nothing (no gate to dodge) and rides a metering endpoint (10%
    // billed incl. a -15.51 mega). Bare serves free. Strip it here so the
    // leg opens bare. Premium (non-free-base) :nitro passes through (that
    // mapping is the only path that 200s for opus-class).
    let model_stripped: String;
    let model_str: &str = {
        let t = model_str.trim();
        match t.rsplit_once(':') {
            Some((base, suf)) if suf.eq_ignore_ascii_case("nitro")
                && framer_core::model::free_on_cutoff(base) =>
            {
                model_stripped = base.to_string();
                &model_stripped
            }
            _ => model_str,
        }
    };
    {
        // Pass the FULL (possibly mapped) id: SUFFIX_FREE matches exact provider-prefixed
        // ids; stripping first would miss ("claude-opus-5.5:nitro" != allowlisted
        // "anthropic/claude-opus-5.5:nitro") and wrongly deny mapped traffic.
        let m = model_str.trim();
        let m = if m.is_empty() { "gpt-4.1-mini" } else { m };
        if !cutoff_free_model(m) {
            return Err(cutoff_model_denied(model_str));
        }
    }
    // SPEND BREAKER: latched => refuse before any pool read/lease/open.
    if framer_custom::ledger::spend_breaker_tripped() {
        return Err(ApiError::ServingCapacity(
            "spend breaker latched (a cutoff leg settled nonzero) -- cutoff serving halted until an operator resets it via POST /api/custom/complete/status {\"reset_spend_breaker\":true}".into(),
        ));
    }
    // ONE request id per CLIENT request (not per failover attempt): every
    // attempt, leg, refund, and trip line for this request shares req_short,
    // so `grep req=<short>` reconstructs the full causal chain. Attempts are
    // numbered (attempt=1..3) to distinguish failover hops sharing the id.
    let req_id = uuid::Uuid::new_v4().to_string();
    let req_short = req_id[..8].to_string();
    let mut tried: Vec<String> = Vec::new();
    let mut last: Option<ApiError> = None;
    for attempt in 1..=3 {
        match custom_real_once(s.clone(), headers, request.clone(), model_str, max_legs, stop, &tried, &req_id, req_short.clone(), attempt).await {
            Ok(out) => {
                // SPEND BREAKER (here, not in a renderer: every renderer --
                // custom SSE, chat, responses -- funnels through cutoff_run).
                if out.assembled.total_debit < -1e-4 {
                    framer_custom::ledger::trip_spend_breaker(out.assembled.total_debit);
                }
                return Ok(out);
            }
            Err((e, Some(failed))) => {
                eprintln!(
                    "[custom-route] account-local failure on {} ({}); failing over to a fresh account",
                    &failed[..8.min(failed.len())],
                    format!("{e:?}").chars().take(160).collect::<String>(),
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
/// ADR-003: serve a text-only chat-completions request through the cutoff
/// chain (the meter never settles) and render the chat dialect. Tools stay
/// on the native chain -- a tool call cannot split across legs -- the
/// dispatcher guarantees a tools-free payload here.
/// The admission contract every route shares (the endpoint tests pin it):
/// wave pause -> Cooling; all-cooling pool -> AccountCooling with the
/// soonest recovery; one state read, zero browser/session I/O. The chain's
/// own selection repeats these per attempt; admission exists so a STREAMING
/// caller gets the error as a response, not a stream that dies silently.
async fn cutoff_admit(s: &AppState) -> Result<(), ApiError> {
    if let Some(remaining) = s.wave.check() {
        return Err(ApiError::Cooling {
            retry_after_secs: remaining.as_secs().saturating_add(1),
        });
    }
    let pool = s.state.list().await.map_err(ApiError::Internal)?;
    let now = framer_core::rotation::now_ms();
    if !pool.iter().any(|a| a.is_available_at(now)) {
        let until = pool
            .iter()
            .filter(|a| a.is_eligible())
            .filter_map(|a| a.cooling_until_ms.filter(|u| *u > now))
            .min();
        if let Some(until) = until {
            return Err(ApiError::AccountCooling {
                retry_after_secs: until.saturating_sub(now).div_ceil(1000).max(1),
            });
        }
    }
    Ok(())
}

pub(super) async fn chat_via_cutoff(
    s: AppState,
    headers: &HeaderMap,
    payload: serde_json::Value,
    wants_stream: bool,
    include_usage: bool,
) -> Result<Response, ApiError> {
    let model = payload["model"].as_str().unwrap_or("gpt-4.1").to_string();
    let prompt_chars: usize = payload["messages"]
        .as_array()
        .map(|ms| ms.iter().map(|m| m["content"].as_str().map(|c| c.len()).unwrap_or(0)).sum())
        .unwrap_or(0);
    if !wants_stream {
        let out =
            cutoff_run(s, headers, framer_custom::ChainRequest::Chat(payload), &model, None, None)
                .await?;
        return Ok(chat_completion_json(&out, &model, include_usage, prompt_chars));
    }
    // Admission BEFORE the stream opens: a cooling/wave rejection must be
    // the 503 response, not a stream that dies after the headers (the
    // endpoint tests pin that contract).
    cutoff_admit(&s).await?;
    // Streaming: the chain runs in a task; the SSE stream pings every 8s of
    // silence (clients time out silent sockets), then replays the answer as
    // chunk deltas and ends. Buffered chain, replayed stream -- the meter is
    // never settled either way.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<CutoffOutcome, ApiError>>(1);
    let model2 = model.clone();
    let headers2 = headers.clone();
    // DISCONNECT KILLS THE CHAIN (#2): a bare spawn outlives a dropped response --
    // dead clients kept opening legs for nobody. AbortOnDrop + retain_owner ties the
    // chain task's life to the response body.
    let handle = AbortOnDrop(tokio::spawn(async move {
        let out =
            cutoff_run(s, &headers2, framer_custom::ChainRequest::Chat(payload), &model2, None, None)
                .await;
        let _ = tx.send(out).await;
    }));
    let id = format!("chatcmpl-{}", uuid::Uuid::new_v4().simple());
    let created = framer_core::rotation::now_ms() / 1000;
    let mut frames: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let stream = futures::stream::unfold(
        (rx, false, frames),
        move |(mut rx, mut done, mut frames)| {
            let id = id.clone();
            let model = model.clone();
            async move {
                if done {
                    return None;
                }
                if let Some(frame) = frames.pop_front() {
                    return Some((Ok::<_, std::convert::Infallible>(frame), (rx, done, frames)));
                }
                match tokio::time::timeout(std::time::Duration::from_secs(8), rx.recv()).await {
                    Ok(Some(Ok(out))) => {
                        let answer = out.assembled.text.clone();
                        let completion_tokens = answer.chars().count() / 4;
                        let prompt_tokens = prompt_chars / 4;
                        frames.push_back(chat_chunk(&id, created, &model, serde_json::json!({"role": "assistant"})));
                        if !out.assembled.tool_calls.is_empty() {
                            frames.push_back(chat_chunk(&id, created, &model, serde_json::json!({"tool_calls": framer_core::convert::tool_calls_openai(&out.assembled.tool_calls)})));
                        }
                        let chars: Vec<char> = answer.chars().collect();
                        for piece in chars.chunks(400) {
                            let piece: String = piece.iter().collect();
                            frames.push_back(chat_chunk(&id, created, &model, serde_json::json!({"content": piece})));
                        }
                        let fin_reason = if out.assembled.tool_calls.is_empty() { "stop" } else { "tool_calls" };
                        let mut finchunk = serde_json::json!({"id": id, "object": "chat.completion.chunk", "created": created, "model": model, "choices": [{"index": 0, "delta": {}, "finish_reason": fin_reason}]});
                        if include_usage {
                            finchunk["usage"] = serde_json::json!({
                                "prompt_tokens": prompt_tokens,
                                "completion_tokens": completion_tokens,
                                "total_tokens": prompt_tokens + completion_tokens,
                            });
                            finchunk["x-framerize-usage"] = serde_json::json!("estimated");
                        }
                        frames.push_back(format!("data: {}\n\n", finchunk));
                        frames.push_back("data: [DONE]\n\n".to_string());
                        done = false; // stays false until frames drain; flip below
                        // pop the first frame now; mark done when frames empty
                        let f = frames.pop_front().unwrap();
                        let d = frames.is_empty();
                        Some((Ok(f), (rx, d, frames)))
                    }
                    Ok(Some(Err(e))) => {
                        done = true;
                        // SSE TERMINAL (2026-11-10): strict chat clients need a
                        // terminal CHUNK (finish_reason=stop, full chunk shape),
                        // not just an error blob + [DONE].
                        let stop_chunk = format!(
                            "data: {}\n\n",
                            serde_json::json!({
                                "id": id, "object": "chat.completion.chunk", "created": created, "model": model,
                                "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                            })
                        );
                        let frame = format!(
                            "data: {}\n\n{}data: [DONE]\n\n",
                            serde_json::json!({"error": {"message": format!("{e:?}"), "type": "api_error"}}),
                            stop_chunk,
                        );
                        Some((Ok(frame), (rx, done, frames)))
                    }
                    Ok(None) => None,
                    Err(_) => Some((Ok(": ping\n\n".to_string()), (rx, done, frames))),
                }
            }
        },
    );
    let stream = retain_owner(stream, handle);
    let mut resp = axum::response::Response::new(axum::body::Body::from_stream(stream));
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/event-stream"),
    );
    Ok(resp)
}

/// ADR-003: serve a responses-API request (codex) through the CHAT cutoff
/// chain -- the finish cutoff is proven free there, tool calls included --
/// converting the request down and rendering the result back into the
/// responses dialect.
pub(super) async fn responses_via_cutoff(
    s: AppState,
    headers: &HeaderMap,
    payload: serde_json::Value,
    wants_stream: bool,
) -> Result<Response, ApiError> {
    let model = payload["model"].as_str().unwrap_or("gpt-4.1").to_string();
    let chat = framer_core::convert::responses_request_to_chat(&payload).map_err(ApiError::InvalidRequest)?;
    if !wants_stream {
        let out = cutoff_run(s, headers, framer_custom::ChainRequest::Chat(chat), &model, None, None).await?;
        return Ok(render_responses(&out, &model, false));
    }
    // Admission before the stream opens (same contract as the chat path).
    cutoff_admit(&s).await?;
    // The chain runs in a task; the stream pings every 8s of silence, then
    // replays the responses-dialect frames.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<CutoffOutcome, ApiError>>(1);
    let headers2 = headers.clone();
    let model2 = model.clone();
    // DISCONNECT KILLS THE CHAIN (#2): a bare spawn outlives a dropped response --
    // dead clients kept opening legs for nobody. AbortOnDrop + retain_owner ties the
    // chain task's life to the response body.
    let handle = AbortOnDrop(tokio::spawn(async move {
        let out = cutoff_run(s, &headers2, framer_custom::ChainRequest::Chat(chat), &model2, None, None).await;
        let _ = tx.send(out).await;
    }));
    let stream = futures::stream::unfold((rx, false), move |(mut rx, done)| {
        let model = model.clone();
        async move {
            if done {
                return None;
            }
            match tokio::time::timeout(std::time::Duration::from_secs(8), rx.recv()).await {
                Ok(Some(Ok(out))) => {
                    let body = responses_frames(&out, &model);
                    Some((Ok::<_, std::convert::Infallible>(body), (rx, true)))
                }
                Ok(Some(Err(e))) => {
                    // SSE TERMINAL (2026-11-10): an error frame WITHOUT a
                    // terminal hangs SSE clients. Always close with a failed
                    // response.completed after the error. The terminal carries
                    // a full response object INCLUDING id (strict clients
                    // parse it into a struct with required id -- a bare
                    // {"status","error"} shape disconnects them with
                    // "failed to parse ResponseCompleted: missing field id").
                    let err_id = format!("resp_{}", uuid::Uuid::new_v4().simple());
                    let frame = format!(
                        "data: {}\n\ndata: {}\n\n",
                        serde_json::json!({"type": "error", "error": {"message": format!("{e:?}")}}),
                        serde_json::json!({"type": "response.completed", "response": {"id": err_id, "object": "response", "model": model, "status": "failed", "output": [], "error": format!("{e:?}")}}),
                    );
                    Some((Ok(frame), (rx, true)))
                }
                Ok(None) => None,
                Err(_) => Some((Ok(": ping\n\n".to_string()), (rx, false))),
            }
        }
    });
    let stream = retain_owner(stream, handle);
    let mut resp = axum::response::Response::new(axum::body::Body::from_stream(stream));
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/event-stream"),
    );
    Ok(resp)
}

/// Render a cutoff outcome as a responses-API answer: the assembled text
/// becomes one completed message item; each tool call becomes a completed
/// function_call item.
/// Anthropic-dialect cutoff (2026-11-11): anthro-family requests WITHOUT tools run the
/// abort-guarded cutoff engine (leg1_native carries the marker as a system message) and
/// render back in anthro shape. The native bundled path reads the terminal BY CONSTRUCTION
/// and bills real money on terminal-settling meters (proved live: fable native -0.24) --
/// the cutoff abort is the only $0 path. Tool-carrying requests stay native (a tool call
/// cannot split across legs). Same admission contract as chat/responses cutoff paths.
pub(super) async fn anthro_via_cutoff(
    s: AppState,
    headers: &HeaderMap,
    anthro: serde_json::Value,
    wants_stream: bool,
) -> Result<Response, ApiError> {
    let model = anthro["model"].as_str().unwrap_or("claude-haiku-4.5").to_string();
    if !wants_stream {
        let out = cutoff_run(
            s,
            headers,
            framer_custom::ChainRequest::Chat(anthro.clone()),
            &model,
            None,
            None,
        )
        .await?;
        let r = framer_core::convert::AssembledResponse {
            id: format!("msg_{}", uuid::Uuid::new_v4().simple()),
            content: framer_core::convert::text_plus_calls(&out.assembled.text, &out.assembled.tool_calls),
            stop_reason: if out.assembled.tool_calls.is_empty() { "end_turn".to_string() } else { "tool_use".to_string() },
            stop_sequence: None,
            usage: serde_json::json!({}),
            input_tokens: 0,
            output_tokens: (out.assembled.text.chars().count() / 4).max(1) as u64,
            chunks: out.assembled.legs.len() as u32,
            metered_chunks: 0,
        };
        let body = framer_core::convert::render_json(
            framer_core::convert::ClientDialect::AnthropicMessages,
            &r,
            &model,
        );
        let mut resp = axum::response::Response::new(axum::body::Body::from(
            serde_json::to_string(&body).map_err(|e| ApiError::Internal(e.to_string()))?,
        ));
        resp.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
        return Ok(resp);
    }
    cutoff_admit(&s).await?;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<CutoffOutcome, ApiError>>(1);
    let model2 = model.clone();
    let headers2 = headers.clone();
    // DISCONNECT KILLS THE CHAIN (#2): a bare spawn outlives a dropped response --
    // dead clients kept opening legs for nobody. AbortOnDrop + retain_owner ties the
    // chain task's life to the response body.
    let handle = AbortOnDrop(tokio::spawn(async move {
        let out = cutoff_run(
            s,
            &headers2,
            framer_custom::ChainRequest::Chat(anthro),
            &model2,
            None,
            None,
        )
        .await;
        let _ = tx.send(out).await;
    }));
    let stream = futures::stream::unfold((rx, false), move |(mut rx, done)| {
        let model = model.clone();
        async move {
            if done {
                return None;
            }
            match tokio::time::timeout(std::time::Duration::from_secs(8), rx.recv()).await {
                Ok(Some(Ok(out))) => {
                    let r = framer_core::convert::AssembledResponse {
                        id: format!("msg_{}", uuid::Uuid::new_v4().simple()),
                        content: framer_core::convert::text_plus_calls(&out.assembled.text, &out.assembled.tool_calls),
                        stop_reason: if out.assembled.tool_calls.is_empty() { "end_turn".to_string() } else { "tool_use".to_string() },
                        stop_sequence: None,
                        usage: serde_json::json!({}),
                        input_tokens: 0,
                        output_tokens: (out.assembled.text.chars().count() / 4).max(1) as u64,
                        chunks: out.assembled.legs.len() as u32,
                        metered_chunks: 0,
                    };
                    let frames = framer_core::convert::render_sse(
                        framer_core::convert::ClientDialect::AnthropicMessages,
                        &r,
                        &model,
                        false,
                    );
                    let body = frames.join("") + "data: [DONE]\n\n";
                    Some((Ok::<_, std::convert::Infallible>(body), (rx, true)))
                }
                Ok(Some(Err(e))) => {
                    let frame = format!(
                        "data: {}\n\ndata: [DONE]\n\n",
                        serde_json::json!({"error": {"message": format!("{e:?}"), "type": "api_error"}})
                    );
                    Some((Ok(frame), (rx, true)))
                }
                Ok(None) => None,
                Err(_) => Some((Ok(": ping\n\n".to_string()), (rx, false))),
            }
        }
    });
    let stream = retain_owner(stream, handle);
    let mut resp = axum::response::Response::new(axum::body::Body::from_stream(stream));
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/event-stream"),
    );
    Ok(resp)
}

fn render_responses(out: &CutoffOutcome, model: &str, stream: bool) -> Response {
    let resp_id = format!("resp_{}", uuid::Uuid::new_v4().simple());
    let created = framer_core::rotation::now_ms() / 1000;
    let mut output: Vec<serde_json::Value> = Vec::new();
    let text = &out.assembled.text;
    if !text.is_empty() {
        output.push(serde_json::json!({
            "id": format!("msg_{}", uuid::Uuid::new_v4().simple()),
            "type": "message",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": text}],
        }));
    }
    for tc in &out.assembled.tool_calls {
        output.push(serde_json::json!({
            "id": format!("fc_{}", uuid::Uuid::new_v4().simple()),
            "type": "function_call",
            "call_id": tc["id"].as_str().unwrap_or(""),
            "name": tc["name"].as_str().unwrap_or(""),
            "arguments": tc["arguments"].as_str().unwrap_or(""),
            "status": "completed",
        }));
    }
    let completion_tokens = text.chars().count() / 4;
    let usage = serde_json::json!({
        "input_tokens": 0,
        "output_tokens": completion_tokens,
        "total_tokens": completion_tokens,
    });
    if !stream {
        return axum::Json(serde_json::json!({
            "id": resp_id,
            "object": "response",
            "created_at": created,
            "status": "completed",
            "model": model,
            "output": output,
            "usage": usage,
            "x-framerize-usage": "estimated",
        }))
        .into_response();
    }
    let body = responses_frames(out, model);
    return ([("content-type", "text/event-stream")], body).into_response();
}

/// The responses-dialect SSE replay: response.created, item added/done per
/// item, response.completed with usage.
fn responses_frames(out: &CutoffOutcome, model: &str) -> String {
    let resp_id = format!("resp_{}", uuid::Uuid::new_v4().simple());
    let created = framer_core::rotation::now_ms() / 1000;
    let mut output: Vec<serde_json::Value> = Vec::new();
    let text = &out.assembled.text;
    if !text.is_empty() {
        output.push(serde_json::json!({
            "id": format!("msg_{}", uuid::Uuid::new_v4().simple()),
            "type": "message",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": text}],
        }));
    }
    for tc in &out.assembled.tool_calls {
        output.push(serde_json::json!({
            "id": format!("fc_{}", uuid::Uuid::new_v4().simple()),
            "type": "function_call",
            "call_id": tc["id"].as_str().unwrap_or(""),
            "name": tc["name"].as_str().unwrap_or(""),
            "arguments": tc["arguments"].as_str().unwrap_or(""),
            "status": "completed",
        }));
    }
    let completion_tokens = text.chars().count() / 4;
    let usage = serde_json::json!({
        "input_tokens": 0,
        "output_tokens": completion_tokens,
        "total_tokens": completion_tokens,
    });
    let mut body = String::new();
    body.push_str(&format!(
        "data: {}\n\n",
        serde_json::json!({"type": "response.created", "response": {"id": resp_id, "object": "response", "created_at": created, "status": "in_progress", "model": model, "output": []}})
    ));
    for (i, item) in output.iter().enumerate() {
        body.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({"type": "response.output_item.added", "output_index": i, "item": item})
        ));
        if item["type"].as_str() == Some("message") {
            let full = item["content"][0]["text"].as_str().unwrap_or("");
            body.push_str(&format!(
                "data: {}\n\n",
                serde_json::json!({"type": "response.output_text.delta", "output_index": i, "content_index": 0, "delta": full})
            ));
        }
        body.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({"type": "response.output_item.done", "output_index": i, "item": item})
        ));
    }
    body.push_str(&format!(
        "data: {}\n\n",
        serde_json::json!({
            "type": "response.completed",
            "response": {"id": resp_id, "object": "response", "created_at": created, "status": "completed", "model": model, "output": output, "usage": usage},
        })
    ));
    body
}

/// One chat.completion.chunk SSE frame.
fn chat_chunk(id: &str, created: u64, model: &str, delta: serde_json::Value) -> String {
    format!(
        "data: {}\n\n",
        serde_json::json!({
            "id": id, "object": "chat.completion.chunk", "created": created, "model": model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": null}],
        })
    )
}

/// Buffered chat.completion render of a chain outcome.
fn chat_completion_json(
    out: &CutoffOutcome,
    model: &str,
    include_usage: bool,
    prompt_chars: usize,
) -> Response {
    let text = &out.assembled.text;
    let completion_tokens = text.chars().count() / 4;
    let prompt_tokens = prompt_chars / 4;
    let has_tools = !out.assembled.tool_calls.is_empty();
    let message = if has_tools {
        serde_json::json!({"role": "assistant", "content": if text.is_empty() { serde_json::Value::Null } else { serde_json::json!(text) }, "tool_calls": framer_core::convert::tool_calls_openai(&out.assembled.tool_calls)})
    } else {
        serde_json::json!({"role": "assistant", "content": text})
    };
    let mut body = serde_json::json!({
        "id": format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()),
        "object": "chat.completion",
        "created": framer_core::rotation::now_ms() / 1000,
        "model": model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": if has_tools { "tool_calls" } else { "stop" },
        }],
    });
    if include_usage {
        body["usage"] = serde_json::json!({
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens,
        });
        body["x-framerize-usage"] = serde_json::Value::String("estimated".into());
    }
    axum::Json(body).into_response()
}

async fn custom_real_once(
    s: AppState,
    headers: &HeaderMap,
    request: framer_custom::ChainRequest,
    model_str: &str,
    body_max_legs: Option<u32>,
    body_stop: Option<u32>,
    tried: &[String],
    req_id: &str,
    req_short: String,
    attempt: u32,
) -> Result<CutoffOutcome, (ApiError, Option<String>)> {
    // A caller-pinned account is strict: its failures never hop silently.
    let pinned = hdr(headers, "x-framerize-account").is_some();
    let local = |account: &framer_core::account::ManagedAccount| -> Option<String> {
        if pinned {
            None
        } else {
            Some(account.id.0.to_string())
        }
    };
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
            "[custom-route] req={req_short} attempt={attempt} account {} skipped (custom open refused it recently)",
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
            "[custom-route] req={req_short} attempt={attempt} account {} skipped (no funded workspace)",
            short_id(&account.id)
        );
        return Err((
            ApiError::ServingCapacity(
                "no funded workspace on the selected account; pick another or credit one".into(),
            ),
            local(&account),
        ));
    }
    eprintln!("[custom-route] req={req_short} attempt={attempt} stage=lease account={}", short_id(&account.id));
    let residency = RequestResidency::new(
        tokio::time::timeout(STAGE_TIMEOUT, lease_account(&s, account.id))
            .await
            .map_err(|_| {
                eprintln!("[custom-route] req={req_short} attempt={attempt} stage=lease timed out after 90s");
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
    eprintln!("[custom-route] req={req_short} attempt={attempt} stage=session account={}", short_id(&account.id));
    let session = tokio::time::timeout(STAGE_TIMEOUT, s.sessions.load(&account.framer_user_id))
        .await
        .map_err(|_| {
            eprintln!("[custom-route] req={req_short} attempt={attempt} stage=session timed out after 90s");
            (
                ApiError::Internal("custom route: session load timed out after 90s".into()),
                local(&account),
            )
        })?
        .map_err(|e| (ApiError::Internal(e), local(&account)))?
        .ok_or_else(|| (ApiError::Internal("account has no session".into()), local(&account)))?;
    // Scope rule: the oracle (in-page credits GET) and the envelope must bill
    // the SAME workspace -- but which workspace is the FUNDED one, not
    // blindly the primary. The old pin-primary rule failed requests on
    // accounts whose primary was dead while a funded extra sat ready
    // (measured live 2026-11-09: evasion's primary read 0.00 with 636
    // credits across its extras; the route refused in front of them).
    // pick_funded_scope skips known-dead/saturated scopes and pairs each
    // team with its own project (the billing pairing bug fix); the preflight
    // below re-verifies the picked scope's live balance before any leg opens.
    eprintln!("[custom-route] req={req_short} attempt={attempt} stage=scope account={}", short_id(&account.id));
    let scope = tokio::time::timeout(STAGE_TIMEOUT, crate::pick_funded_scope(&s, &account, &session))
        .await
        .map_err(|_| {
            eprintln!("[custom-route] req={req_short} attempt={attempt} stage=scope timed out after 90s");
            (
                ApiError::Internal("custom route: scope pick timed out after 90s".into()),
                local(&account),
            )
        })?
        .map_err(|e| (e, local(&account)))?;
    let project = scope.project.clone();
    let scope_team = scope.team.clone();
    let chat_session =
        hdr(headers, "x-framerize-session").unwrap_or_else(|| stable_session(&account.id));

    // Per (workspace x MODEL) concurrency, mirroring native_complete.
    // Model id rule (models.rs + live PROOF legs 21:26Z which streamed $0
    // with model=gpt-4.1): the /ai/v3/* gateway takes the UNPREFIXED form.
    // Strip any caller-supplied provider prefix at the boundary so the
    // envelope always carries the gateway form, whatever T3 sends.
    let model = {
        let m = model_str.trim();
        // Empty callers land on the free mini (gpt-4.1 full-size is
        // per-token metered and guarded above).
        let m = if m.is_empty() { "gpt-4.1-mini" } else { m };
        // Do NOT strip provider prefixes here (2026-11-11 -- the opus bug): cutoff_run
        // already mapped bare ids to their canonical :nitro/:floor forms upstream; stripping
        // "anthropic/claude-opus-5.5:nitro" to "claude-opus-5.5:nitro" breaks the exact-match
        // suffix allowlist AND drops the suffix in wrap(), sending the BLOCKED base id.
        m.to_string()
    };
    // Gated/blocked models fail FAST, before any account burn (the cutoff
    // path learned this from the native one: the gpt-6-astra churn burned a
    // page open per account per request). The gate is keyed on the gateway
    // form the custom route actually sends.
    let gate_model = framer_core::convert::to_openrouter_id(&model);
    if framer_core::rotation::is_model_gated(&gate_model) {
        return Err((
            ApiError::InvalidRequest(format!(
                "model {model} is unavailable on your workspaces right now (learned from a live refusal; it retries on its own shortly). Working free models: gpt-4.1-mini, claude-haiku-4.5, claude-fable-5.2."
            )),
            None,
        ));
    }
    // SPEND LOCK: single source of truth is cutoff_free_model() (enforced
    // again inside cutoff_run for every renderer). This check fires before
    // any lease/page/open with the same allowlist.
    {
        // Pass the FULL id (mapped or suffixed): SUFFIX_FREE matches exact
        // provider-prefixed ids; stripping first wrongly denies mapped traffic.
        // (The bare-base fast path stays: normalize inside free_on_cutoff.)
        let m = model_str.trim();
        let m = if m.is_empty() { "gpt-4.1-mini" } else { m };
        if !cutoff_free_model(m) {
            return Err((cutoff_model_denied(model_str), None));
        }
    }
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
            eprintln!("[custom-route] req={req_short} attempt={attempt} gate wait timed out (key={gate_key})");
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
    eprintln!("[custom-route] req={req_short} attempt={attempt} stage=bearer account={}", short_id(&account.id));
    let bearer = tokio::time::timeout(STAGE_TIMEOUT, s.client.custom_bearer(&session, &account.id))
        .await
        .map_err(|_| {
            eprintln!("[custom-route] req={req_short} attempt={attempt} stage=bearer timed out after 90s");
            (
                ApiError::Internal("custom route: bearer mint timed out after 90s".into()),
                local(&account),
            )
        })?
        .map_err(|e| (ApiError::Internal(e), local(&account)))?;
    // Seed the browser cookie jar (mirrors attempt: imported accounts start
    // with an empty jar; in-page fetches authenticate from it).
    eprintln!("[custom-route] req={req_short} attempt={attempt} stage=seed account={}", short_id(&account.id));
    tokio::time::timeout(
        STAGE_TIMEOUT,
        s.browser.seed_session(&account.id, &session.cookie_header),
    )
    .await
    .map_err(|_| {
        eprintln!("[custom-route] req={req_short} attempt={attempt} stage=seed timed out after 90s");
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
    // Bound here (not in the block) so the terminal proof line can cite it.
    let open_ledger = s
        .client
        .usage(&session, &scope_team)
        .await
        .map(|snap| snap.available)
        .unwrap_or(f64::NAN);
    {
        let live_credits = open_ledger;
        eprintln!(
            "[custom-route] req={req_short} attempt={attempt} stage=ledger account={} team={} live_credits={:.2}",
            &account.id.0.to_string()[..8.min(account.id.0.to_string().len())],
            &scope_team.0[..8.min(scope_team.0.len())],
            live_credits,
        );
        // LEDGER SYNC (fix): persist the live balance into the local DB ledger so local==live.
        let _ = s.state.set_credit_available(account.id, live_credits).await;

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
        "[custom-route] req={req_short} attempt={attempt} stage=open account={} model={model_log}",
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
                "chain: engine run exceeded {}s with no terminal or stop (req {req_short} attempt {attempt})",
                ENGINE_TIMEOUT.as_secs()
            )),
            None,
        )
    })?;
    let result = residency.observe_result(account.id, result);
    record_account_health(&s, &account, &result).await;
    // GRANT-BUCKET ROUTING (2026-11-15): a billed quarantine means this
    // account's upstream grant bucket is depleted (billed legs cluster in
    // streaks: ef81a070 6-long). An Ok-with-debit must NOT clear health —
    // cool the account so the selector routes away until the bucket refills.
    // (Quarantine text is already withheld; this only steers FUTURE legs.
    // Runs AFTER record_account_health: its Ok arm would note_success-wipe
    // this cooling, so this upsert must win.)
    if let Ok(ref assembled) = result {
        if assembled.total_debit < -framer_custom::ledger::LEDGER_EPS {
            let now = framer_core::rotation::now_ms();
            // Re-read: record may have mutated stored health; base off latest.
            if let Ok(Some(latest)) = s.state.get(account.id).await {
                let mut hot = latest;
                hot.note_risk_refused(now);
                let ms = framer_core::account::ManagedAccount::cooldown_backoff_ms(hot.cooldown_step - 1);
                let id8 = hot.id.0.to_string()[..8].to_string();
                let debit = assembled.total_debit;
                let _ = s.state.upsert(&hot).await;
                eprintln!(
                    "[health] {id8}: billed quarantine ({debit:+.6}) -> cooling {ms}ms (grant-bucket backoff)",
                );
            }
        }
    }
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
                // NEWEST-FIRST (#9: turns push oldest-first, but the settler is usually the
                // latest leg; take(4) was refunding the 4 oldest. Same 4-post quota bound,
                // aimed at the actual settler. Refunds deny ~261:2 historically -- this is
                // last-resort hygiene, never the plan.)
                let rev: Vec<String> = turns.iter().rev().cloned().collect();
                for turn_id in rev.iter().take(4) {
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
                                "[custom-route] req={req_short} attempt={attempt} refund turn {} GRANTED (settle reversed)",
                                &turn_id[..8.min(turn_id.len())]
                            );
                            // Stop when whole: quota is ~2/window, so a recovered balance
                            // must not burn remaining posts (each extra is quota spent).
                            if let Ok(snap) = s.client.usage(&refund_session, &scope_team).await {
                                if snap.available >= before {
                                    eprintln!(
                                        "[custom-route] req={req_short} attempt={attempt} balance whole after {refunded} grant(s); stopping refund loop"
                                    );
                                    break;
                                }
                            }
                        }
                        Ok((status, _, text)) => eprintln!(
                            "[custom-route] req={req_short} attempt={attempt} refund turn {} denied/HTTP {status}: {}",
                            &turn_id[..8.min(turn_id.len())],
                            &text[..text.len().min(120)]
                        ),
                        Err(e) => eprintln!(
                            "[custom-route] req={req_short} attempt={attempt} refund turn {} transport: {e}",
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
                                "[custom-route] req={req_short} attempt={attempt} refunds recovered {recovered:+.6}; adjusted total_debit={:+.6}",
                                assembled.total_debit
                            );
                        }
                    }
                }
                // SWEEPER: journal this chain's turns if debit remains; retry aged entries
                // (free posts) so the next quota burst recovers old settles automatically.
                // Aged 7 days: older turns are presumed unrefundable; journal stays small.
                if assembled.total_debit < -1e-6 {
                    for turn_id in turns.iter() {
                        sweeper_append(&account.id.0.to_string(), turn_id, assembled.total_debit);
                    }
                    let aged = sweeper_take(7 * 24 * 3600 * 1000);
                    for (acc, turn, _debit) in aged.iter().take(2) {
                        if acc != &account.id.0.to_string() {
                            continue; // this cycle refunds on its own account only
                        }
                        let body = serde_json::json!({
                            "projectId": refund_project.0.as_str(),
                            "featureCategory": refund_category.as_str(),
                            "featureName": if refund_category == "translation" { "translation" } else { "agents" },
                            "step": refund_step.as_str(),
                            "sessionId": chat_session.as_str(),
                            "stepIndex": 0,
                            "turnId": turn.as_str(),
                            "userPrompt": "",
                            "castleToken": "garbage",
                        });
                        match s.browser.ai_post(&account.id, "/ai/v3/refund/", &refund_bearer, &body).await {
                            Ok((200, _, text))
                                if text.contains("\"granted\":true")
                                    || text.contains("\"granted\": true") =>
                            {
                                eprintln!("[sweeper] aged turn {} GRANTED (burst captured)", &turn[..8.min(turn.len())]);
                                sweeper_drop(&account.id.0.to_string(), turn);
                                refunded += 1;
                            }
                            _ => {}
                        }
                    }
                } else {
                    // Whole: drop this chain's turns if previously journaled (e.g. re-run).
                    for turn_id in turns.iter() {
                        sweeper_drop(&account.id.0.to_string(), turn_id);
                    }
                }
            }
            // PROOF LINE: ledger re-read at close. Each request's own delta is
            // computable from its stage=ledger line (open) and close_ledger here
            // — no paired external reads needed. team+project identify the scope.
            let close_ledger = s.client.usage(&session, &scope_team).await
                .map(|snap| snap.available).unwrap_or(f64::NAN);
            eprintln!(
                "[custom-route] req={req_short} attempt={attempt} account={} model={} legs={} chars={} total_debit={:+.6} refunded={} outcome={:?} session={} team={} project={} open_ledger={:.2} close_ledger={:.2}",
                    &account.id.0.to_string()[..8.min(account.id.0.to_string().len())],
                model_log.clone(),
                assembled.legs.len(),
                assembled.text.len(),
                assembled.total_debit,
                refunded,
                assembled.outcome,
                chat_session,
                &scope_team.0[..8.min(scope_team.0.len())],
                &scope.project.0.to_string()[..8.min(scope.project.0.to_string().len())],
                open_ledger,
                close_ledger,
            );
            Ok(CutoffOutcome {
                assembled,
                req_id: req_id.to_string(),
                chat_session,
                refunded,
            })
        }
        Err(e) => {
            eprintln!("[custom-route] req={req_short} attempt={attempt} account={} model={} FAILED: {}",
                    &account.id.0.to_string()[..8.min(account.id.0.to_string().len())],
                model_log,
                e.chars().take(300).collect::<String>());
            if e.contains("retryable 429") {
                s.gate.on_overloaded(&gate_key).await;
                if let Some(team) = account.current_team.as_ref() {
                    framer_core::rotation::mark_saturated(&team.0, 45_000);
                }
            }
            // A credits-exhausted scope means the leased workspace is truly out of
            // credits (provider-verified, not a guess). Mark the scope team
            // saturated for 5 minutes so later picks steer away from the dead team.
            // NOTE: match on the structured marker ONLY. The old `e.contains("402")`
            // substring fired on 403 entitlement envelopes whose timestamps/refs
            // contain "402" (e.g. "2026-11-15T04:20:09.07340246Z"), wrongly
            // saturating FUNDED workspaces (b4d2e736 399cr, 92775550 103cr) for
            // 5min and starving the pool behind a false "saturated" failover loop.
            if e.ends_with("credits-exhausted")
                || e.contains("credit_balance_exhausted")
                || e.contains("team_credit_balance_exhausted")
            {
                framer_core::rotation::mark_saturated(&scope_team.0, 300_000);
            }
            // An OPEN refusal (leg 1 never streamed) is this account's
            // custom-open pattern being risk-hot: mark it (custom-only --
            // the account keeps serving other routes) and fail over. A
            // failure after legs opened returns as-is.
            // Mark only a genuine Castle risk refusal. A guardrail 404
            // ("0 endpoints") or an entitlement 403 is NOT account heat --
            // marking those blocked FUNDED accounts from the whole cutoff
            // path for 10min over one gated model (measured live: the
            // gpt-6-astra churn marked reis+evasion and the route died).
            let is_castle = e.contains("blocked by our security checks")
                || e.contains("request_blocked")
                || e.contains("risk-refused");
            let retry = if e.contains("leg 1 failed")
                && e.contains("page stream open failed")
                && is_castle
            {
                mark_open_refused(&account.id.0.to_string());
                local(&account)
            } else if e.contains("leg 1 failed") && e.contains("page stream open failed") {
                // Non-Castle open failure (guardrail/entitlement): account is
                // fine; fail over without marking.
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
            Json(CompleteBody { prompt: "explain X".into(), model: "gpt-4.1-mini".into(), max_legs: None , stop: None}),
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
        assert!(open.contains("openai/gpt-4.1"), "catalog id (OpenRouter form, same as native legs): {open}");
        assert!(open.contains("explain X"), "leg prompt inside request: {open}");
        assert!(open.contains("request"), "native payload nested under request: {open}");
        assert!(open.contains(", 200)"), "rung stop baked into page open: {open}");
        drop(seen);
        // Session jar seeded exactly once (mirrors attempt).
        let seeded = ports.seeded.lock().unwrap();
        assert_eq!(seeded.as_slice(), ["session=custom-fixture"]);
    }

    #[test]
    fn spend_lock_allowlist_is_mini_plus_haiku_only() {
        assert!(cutoff_free_model("gpt-4.1-mini"));
        assert!(cutoff_free_model("claude-haiku-4.5"));
        assert!(cutoff_free_model("openai/gpt-4.1-mini"));
        assert!(!cutoff_free_model("gpt-4.1"));
        assert!(!cutoff_free_model("kimi-k3"));
        assert!(!cutoff_free_model("moonshotai/kimi-k3"));
        assert!(!cutoff_free_model("gemini-2.5-flash"));
        assert!(!cutoff_free_model("deepseek-chat"));
        assert!(!cutoff_free_model("grok-4"));
        assert!(!cutoff_free_model("gpt-6-sol"));
        assert!(!cutoff_free_model(""));
    }

    #[tokio::test]
    async fn spend_lock_denies_before_any_io() {
        // A denied model returns InvalidRequest with ZERO page evals.
        // Serialized: the breaker trip/reset test mutates the same global.
        // NOTE: bare kimi-k3 is MAPPED (not denied) since the floor-mapping shipped;
        // use deepseek-chat (per-token, unmapped,_suffix-free) as the denied probe.
        let _env = ENV_LOCK.lock().unwrap();
        framer_custom::ledger::reset_spend_breaker();
        let (state, ports) = test_state_with(Vec::new());
        let resp = custom_complete_handler(
            State(state),
            HeaderMap::new(),
            Json(CompleteBody { prompt: "hi".into(), model: "deepseek-chat".into(), max_legs: None, stop: None }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(ports.eval_seen.lock().unwrap().is_empty(), "no page eval on denied model");
    }

    #[test]
    fn spend_breaker_trips_refuses_resets() {
        let _env = ENV_LOCK.lock().unwrap();
        framer_custom::ledger::reset_spend_breaker();
        assert!(!framer_custom::ledger::spend_breaker_tripped());
        // Dust (at/below floor) quarantines the request WITHOUT tripping.
        framer_custom::ledger::trip_spend_breaker(-0.05);
        assert!(!framer_custom::ledger::spend_breaker_tripped());
        // Large settles trip.
        framer_custom::ledger::trip_spend_breaker(-2.71);
        assert!(framer_custom::ledger::spend_breaker_tripped());
        framer_custom::ledger::reset_spend_breaker();
        assert!(!framer_custom::ledger::spend_breaker_tripped());
    }

    #[tokio::test]
    async fn real_mode_rejects_empty_prompt_before_io() {
        let (state, ports) = test_state_with(Vec::new());
        let resp = custom_complete_handler(
            State(state),
            HeaderMap::new(),
            Json(CompleteBody { prompt: "  ".into(), model: "gpt-4.1-mini".into(), max_legs: None , stop: None}),
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
            Json(CompleteBody { prompt: "explain X".into(), model: "gpt-4.1-mini".into(), max_legs: None , stop: None}),
        )
        .await;
        // Unknown account pin: no eligible account, no page touched.
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(ports.eval_seen.lock().unwrap().is_empty(), "no page eval without admission");
    }
}
