//! Generic dialect proxy — the compat layer's engine. Framer's gateway natively
//! speaks OpenAI Responses, OpenAI chat/completions (OpenRouter), and Anthropic
//! Messages; every call is the same Gq envelope with `request` = the native
//! payload forwarded nearly verbatim. Verified live 2026-10-01, see
//! reference/analysis/verified-captures-2026-10-01.md.
//!
//! Metering (RESOLVED 2026-10-01, verified-captures §6): responses + anthropic
//! routes are metered. The OpenRouter chat route is metered ONLY when
//! `stream:true` — `stream:false` never debits (the meter hooks the SSE path).
//! Free long-form generation: see freechat.rs (continuation chaining around the
//! ~21.5 s idle-connection kill on held non-streaming requests).

use crate::auth::{self, AiToken};
use crate::transport::Transport;
use framer_core::ids::{AccountId, ProjectId};
use framer_core::port::{BrowserBridge, Session};
use serde_json::{json, Value};
use std::sync::Mutex;

#[cfg(test)]
#[path = "proxy_forward_contract_tests.rs"]
mod forward_contract_tests;

#[derive(Clone, Copy, Debug)]
pub enum Dialect {
    /// /ai/v3/openai/responses/ — unprefixed model ids ("gpt-6-sol"). Metered.
    OpenAiResponses,
    /// /ai/v3/chat/ — OpenRouter catalog ids ("openai/gpt-4.1"). Metering uncertain.
    OpenRouterChat,
    /// /ai/v3/anthropic/messages/ — Anthropic ids ("claude-sonnet-4-5"). Metered.
    AnthropicMessages,
    /// /ai/v3/messages/ — Anthropic format via OpenRouter.
    OpenRouterMessages,
    /// /ai/v3/responses/ — OpenAI Responses format via OpenRouter. Free
    /// non-streaming (verified 2026-10-05); the codex-native chain route.
    OpenRouterResponses,
}

impl Dialect {
    pub fn path(self) -> &'static str {
        // Single source of truth: the route registry (routes/mod.rs).
        crate::routes::by_dialect(self).path
    }
}

/// Forward one native payload through the gateway. The request is sent
/// IN-PAGE from the account's own browser (`BrowserBridge::ai_post`), which
/// also mints the Castle token there: the verified rule (2026-10-02 quadrant
/// matrix) is that token and transport must share one environment — the same
/// envelope via reqwest gets risk-refused even with a perfect token.
/// Returns the upstream response buffered (SSE consumed whole in-page) as a
/// ProxyResponse. One 401 retry with a fresh JWT, mirroring inference.rs.
#[allow(clippy::too_many_arguments)]
pub async fn forward(
    t: &Transport,
    token_cache: &Mutex<Vec<(AccountId, AiToken)>>,
    session: &Session,
    browser: &dyn BrowserBridge,
    account: &AccountId,
    project: &ProjectId,
    dialect: Dialect,
    step: &str,
    payload: Value,
    chat_session: &str,
    step_index: u32,
) -> Result<framer_core::port::ProxyResponse, String> {
    forward_cancellable(t, token_cache, session, browser, account, project,
        dialect, step, payload, chat_session, step_index, None).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn forward_cancellable(
    t: &Transport, token_cache: &Mutex<Vec<(AccountId, AiToken)>>,
    session: &Session, browser: &dyn BrowserBridge, account: &AccountId,
    project: &ProjectId, dialect: Dialect, step: &str, payload: Value,
    chat_session: &str, step_index: u32,
    cancelled: Option<&(dyn Fn() -> bool + Send + Sync)>,
) -> Result<framer_core::port::ProxyResponse, String> {
    forward_with_category(t, token_cache, session, browser, account, project,
        dialect, step, payload, chat_session, step_index, cancelled, None).await
}

pub(crate) fn category_from_env() -> String {
    std::env::var("FRAMERIZE_FEATURE_CATEGORY").unwrap_or_else(|_| "translation".to_string())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn forward_with_category(
    t: &Transport, token_cache: &Mutex<Vec<(AccountId, AiToken)>>,
    session: &Session, browser: &dyn BrowserBridge, account: &AccountId,
    project: &ProjectId, dialect: Dialect, step: &str, payload: Value,
    chat_session: &str, step_index: u32,
    cancelled: Option<&(dyn Fn() -> bool + Send + Sync)>, category: Option<&str>,
) -> Result<framer_core::port::ProxyResponse, String> {
    check_cancelled(cancelled)?;
    let t0 = std::time::Instant::now();
    let req_id = uuid::Uuid::new_v4().to_string();
    let token = cached_token(t, token_cache, session, account).await?;
    check_cancelled(cancelled)?;
    let category = category.map(str::to_owned).unwrap_or_else(category_from_env);
    match attempt(browser, session, account, &token, project, dialect, step, &payload, chat_session, &category, step_index, cancelled).await {
        Ok(resp) => {
            framer_core::observability::Event::new("ai.request")
                .request_id(&req_id)
                .account(account)
                .field("dialect", format!("{dialect:?}"))
                .field("streaming", payload["stream"].as_bool() == Some(true))
                .field("status", resp.status)
                .duration_ms(framer_core::observability::elapsed_ms(t0))
                .outcome("ok")
                .emit();
            Ok(resp)
        }
        Err(e) if e == "unauthorized" => {
            check_cancelled(cancelled)?;
            auth::invalidate_ai_token(t, session, &token);
            if let Ok(mut guard) = token_cache.lock() {
                guard.retain(|(id, _)| id != account);
            }
            let fresh = cached_token(t, token_cache, session, account).await?;
            attempt(browser, session, account, &fresh, project, dialect, step, &payload, chat_session, &category, step_index, cancelled).await
        }
        Err(e) if e == "category-rejected" => {
            check_cancelled(cancelled)?;
            // Structured event so rejected models are visible via
            // `framerize logs --event category.rejected` — that's the signal
            // for pruning a model off the translation path.
            framer_core::observability::Event::new("category.rejected")
                .request_id(&req_id)
                .account(account)
                .field("dialect", format!("{dialect:?}"))
                .field("model", payload["model"].as_str().unwrap_or("?"))
                .field("category", category.clone())
                .field("fallback", "agents")
                .duration_ms(framer_core::observability::elapsed_ms(t0))
                .outcome("ok")
                .emit();
            eprintln!("[proxy] model {} rejected for category {category} — retrying as agents", payload["model"].as_str().unwrap_or("?"));
            attempt(browser, session, account, &token, project, dialect, step, &payload, chat_session, "agents", step_index, cancelled).await
        }
        Err(e) => {
            framer_core::observability::Event::new("ai.request")
                .request_id(&req_id)
                .account(account)
                .field("dialect", format!("{dialect:?}"))
                .field("streaming", payload["stream"].as_bool() == Some(true))
                .duration_ms(framer_core::observability::elapsed_ms(t0))
                .error(&e)
                .emit();
            Err(e)
        }
    }
}

fn check_cancelled(cancelled: Option<&(dyn Fn() -> bool + Send + Sync)>) -> Result<(), String> {
    if cancelled.is_some_and(|check| check()) { Err("client cancelled".into()) } else { Ok(()) }
}

#[allow(clippy::too_many_arguments)]
async fn attempt(
    browser: &dyn BrowserBridge,
    session: &Session,
    account: &AccountId,
    token: &AiToken,
    project: &ProjectId,
    dialect: Dialect,
    step: &str,
    payload: &Value,
    chat_session: &str,
    category: &str,
    step_index: u32,
    cancelled: Option<&(dyn Fn() -> bool + Send + Sync)>,
) -> Result<framer_core::port::ProxyResponse, String> {
    check_cancelled(cancelled)?;
    // featureCategory (THE VELOCITY GOOSE, verified 2026-10-05): the
    // "translation" category is accepted on all inference routes, keeps
    // non-streaming FREE (ledger delta 0 across the roster), and has a
    // dramatically larger velocity budget — 240 requests @183/min sustained
    // with zero 403s, where "agents" trips ~120/min (site-wide translation
    // needs hundreds of rapid calls by design, so its budget is generous).
    // Counters are separate: hammering translation never touched the agents
    // budget (healthchecks stayed 200). Fallback: if a model is rejected
    // for the category, retry once as "agents".
    // featureName must match the category (verified in the decompiled client:
    // real translation traffic sends translation/translation/translate —
    // our agents-mismatched meta was an inconsistency a reviewer would spot).
    let feature_name = if category == "translation" { "translation" } else { "agents" };
    // userPrompt (invisibility): the real client sends the user's text on
    // stepIndex 0 — every one of our turns carried "" (visible in any
    // session-history view). Populate from the last user message, clipped
    // like a UI would. top-level `model` also mirrors the real envelope (gf()).
    // Reads both native shapes: Anthropic `messages` and Responses `input`
    // (the codex-native path sends the latter — an empty userPrompt on every
    // turn was the anomaly this extraction exists to avoid).
    let last_user_text = |items: &Vec<Value>| -> Option<String> {
        items.iter().rev().find(|m| m["role"] == "user").map(|m| match &m["content"] {
            Value::String(s) => s.clone(),
            Value::Array(bs) => bs.iter()
                .filter_map(|b| b["text"].as_str().or_else(|| b["input_text"].as_str()))
                .collect::<Vec<_>>().join(" "),
            _ => String::new(),
        })
    };
    let user_prompt: String = payload["messages"]
        .as_array()
        .and_then(last_user_text)
        .or_else(|| payload["input"].as_array().and_then(last_user_text))
        .or_else(|| payload["input"].as_str().map(str::to_string))
        .unwrap_or_default()
        .chars().take(300).collect();
    let turn_id = crate::inference::mint_id_pub();
    let body = json!({
        "projectId": project.0,
        "featureCategory": category,
        "featureName": feature_name,
        "model": payload["model"],
        "step": step,
        "sessionId": chat_session,
        "stepIndex": step_index,
        "turnId": turn_id,
        "userPrompt": user_prompt,
        // castleToken is added in-page at fetch time (fresh mint, same env).
        "request": payload,
    });
    // Imported accounts have an empty browser cookie jar — seed the session.
    browser.seed_session(account, &session.cookie_header).await?;
    check_cancelled(cancelled)?;
    // Timed because latency is the tiebreaker when a 403 body says nothing:
    // Framer's edge refuses without proxying (~250ms), the upstream cannot.
    let sent_at = std::time::Instant::now();
    let (status, usage_header, text) = browser
        .ai_post_cancellable(account, dialect.path(), &token.bearer, &body, cancelled)
        .await?;
    // X-Framer-AI-Usage rides every successful AI response (free real-time
    // balance telemetry — no extra call). It was captured and discarded as
    // `_usage` everywhere (audit 2026-10-11); log it so balances are visible
    // per request without polling the rate-limited ledger endpoint.
    if let Some(remaining) = usage_header.as_deref().and_then(crate::credits::parse_usage_header) {
        framer_core::observability::Event::new("credits.usage_header")
            .account(account)
            .field("remaining", remaining)
            .field("status", status)
            .outcome("ok")
            .emit();
    }
    let elapsed = sent_at.elapsed();
    let streaming = payload["stream"].as_bool() == Some(true);
    check_response_status(status, &text, dialect.path(), category, elapsed.as_millis() as u64)?;
    // Inference never requests an automatic billing refund.
    let content_type = if streaming { "text/event-stream" } else { "application/json" }.to_string();
    // Our own queue time, reported so the chunk sizer can exclude it (see
    // ProxyResponse::queue_wait_ms).
    let queue_wait_ms = browser.last_queue_wait_ms(account).await;
    Ok(framer_core::port::ProxyResponse {
        status,
        content_type,
        stream: Box::pin(futures::stream::once(async move {
            Ok::<bytes::Bytes, String>(bytes::Bytes::from(text))
        })),
        queue_wait_ms,
    })
}

fn check_response_status(status: u16, text: &str, path: &str, category: &str, elapsed_ms: u64) -> Result<(), String> {
    match status {
        200..=299 => Ok(()),
        401 => Err("unauthorized".into()),
        402 => Err("credits-exhausted".into()),
        403 => Err(crate::classify::classify_403(text, elapsed_ms).as_error(path, text)),
        // This is the browser's local cooldown, not an upstream workspace 429.
        // Let the existing account failover choose another admitted account.
        0 if text.starts_with("velocity-paused:") => {
            // The sidecar rounds down its remaining wait. Keep the account
            // parked through that window without reporting a new refusal.
            let retry_after_secs = text.rsplit_once("; retry in ")
                .and_then(|(_, wait)| wait.strip_suffix('s'))
                .and_then(|wait| wait.parse::<u64>().ok())
                .map(|wait| wait.saturating_add(1))
                .unwrap_or(60);
            let reason = text.strip_prefix("velocity-paused: ").unwrap_or(text);
            let reason = reason.rsplit_once("; retry in ").map_or(reason, |(reason, _)| reason);
            Err(format!("account-cooling:{retry_after_secs}; {}",
                framer_core::error::safe_failure_summary(reason)))
        }
        400 if category != "agents" && text.contains("not allowed for this feature") =>
            Err("category-rejected".into()),
        _ => Err(format!("retryable {status} on {path}: {}", text.chars().take(500).collect::<String>())),
    }
}

async fn cached_token(
    t: &Transport,
    cache: &Mutex<Vec<(AccountId, AiToken)>>,
    session: &Session,
    account: &AccountId,
) -> Result<AiToken, String> {
    // Per-account cache (2026-10-05): the old SINGLE global slot meant every
    // account rotation invalidated the previous chain's JWT and forced a
    // remint at /auth/ai/access-token — ~1 mint per chain. Under a 300-chain
    // burst that thrash trips the auth endpoint's sustained-rate limiter
    // (measured: 120/120 mints -> 403), which transport classified as Castle
    // "risk-refused" (elapsed=0), poisoning account health with phantom
    // refusals. Per-account caching cuts mints ~99%.
    if let Ok(guard) = cache.lock() {
        if let Some((_, tok)) = guard.iter().find(|(id, _)| id == account) {
            if tok.usable_for(session) {
                return Ok(tok.clone());
            }
        }
    }
    let tok = auth::ai_access_token(t, session).await?;
    if let Ok(mut guard) = cache.lock() {
        guard.retain(|(id, _)| id != account);
        guard.push((*account, tok.clone()));
    }
    Ok(tok)
}

#[cfg(test)]
mod tests {
    use super::check_response_status;

    #[test]
    fn local_device_cooldown_preserves_the_cause_and_wait() {
        let result = check_response_status(0,
            "velocity-paused: every device is cooling; retry in 583s",
            "/ai/v3/responses/", "translation", 0);
        assert_eq!(result.unwrap_err(), "account-cooling:584; every device is cooling");
    }

    #[test]
    fn unrecognized_local_cooldown_keeps_a_bounded_account_wait() {
        let result = check_response_status(0, "velocity-paused: unavailable",
            "/ai/v3/responses/", "translation", 0);
        assert_eq!(result.unwrap_err(), "account-cooling:60; unavailable");
    }

    #[test]
    fn real_workspace_rate_limits_keep_their_retry_policy() {
        let result = check_response_status(429, "workspace full",
            "/ai/v3/responses/", "translation", 40);
        assert_eq!(result.unwrap_err(), "retryable 429 on /ai/v3/responses/: workspace full");
    }

    #[test]
    fn unrelated_transport_failures_do_not_mark_an_account_unhealthy() {
        let result = check_response_status(0, "network timeout",
            "/ai/v3/responses/", "translation", 60_000);
        assert_eq!(result.unwrap_err(), "retryable 0 on /ai/v3/responses/: network timeout");
    }
}
