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
    let t0 = std::time::Instant::now();
    let req_id = uuid::Uuid::new_v4().to_string();
    let token = cached_token(t, token_cache, session, account).await?;
    let category = std::env::var("FRAMERIZE_FEATURE_CATEGORY")
        .unwrap_or_else(|_| "translation".to_string());
    match attempt(browser, session, account, &token, project, dialect, step, &payload, chat_session, &category, step_index).await {
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
            if let Ok(mut guard) = token_cache.lock() {
                guard.retain(|(id, _)| id != account);
            }
            let fresh = cached_token(t, token_cache, session, account).await?;
            attempt(browser, session, account, &fresh, project, dialect, step, &payload, chat_session, &category, step_index).await
        }
        Err(e) if e == "category-rejected" => {
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
            attempt(browser, session, account, &token, project, dialect, step, &payload, chat_session, "agents", step_index).await
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
) -> Result<framer_core::port::ProxyResponse, String> {
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
    // Timed because latency is the tiebreaker when a 403 body says nothing:
    // Framer's edge refuses without proxying (~250ms), the upstream cannot.
    let sent_at = std::time::Instant::now();
    let (status, _usage, text) = browser
        .ai_post(account, dialect.path(), &token.bearer, &body)
        .await?;
    let elapsed = sent_at.elapsed();
    let streaming = payload["stream"].as_bool() == Some(true);
    // Status policy mirrors transport::check (in-page fetch bypasses it).
    match status {
        200..=299 => {}
        401 => return Err("unauthorized".into()),
        402 => return Err("credits-exhausted".into()),
        // A 403 here may be Framer's edge (account health) or the upstream
        // provider refusing the generation (not account health). Classifying
        // every 403 as the former drove 1,399 false cooldowns on 2026-10-04 --
        // see classify.rs.
        403 => {
            let r = crate::classify::classify_403(&text, elapsed.as_millis() as u64);
            return Err(r.as_error(dialect.path(), &text));
        }
        s => {
            let clipped = &text[..text.len().min(500)];
            // Model not on the translation allowlist → caller retries as agents.
            if s == 400 && text.contains("not allowed for this feature") && category != "agents" {
                return Err("category-rejected".into());
            }
            // velocity-paused (sidecar: every device cooling, wait >60s) — map
            // onto the 429 backoff path so chains wait-and-retry instead of
            // erroring into client retry storms (the 2026-10-06 cascade).
            if s == 0 && text.contains("velocity-paused") {
                return Err(format!("retryable 429 on {}: {clipped}", dialect.path()));
            }
            return Err(format!("retryable {s} on {}: {clipped}", dialect.path()));
        }
    }
    // Auto-refund (verified 2026-10-05): /ai/v3/refund/ has NO Castle layer
    // and fully reverses a streamed charge — `{"granted":true}` restores the
    // exact debit. Idempotent (second refund on the same turn is denied), and
    // each account has a small grant quota (~2/window; main exhausted it
    // during verification, acct2's first-ever refund granted). Pure upside:
    // denied costs exactly what the stream already cost. The full body is
    // already awaited above, so the stream has closed and the charge posted.
    if streaming {
        let refund_body = json!({
            "projectId": project.0,
            "featureCategory": category,
            "featureName": feature_name,
            "step": step,
            "sessionId": chat_session,
            "stepIndex": 0,
            "turnId": turn_id,
            "userPrompt": "",
            "castleToken": "garbage", // route ignores it (verified)
        });
        match browser.ai_post(account, crate::routes::by_id("refund").map(|r| r.path).unwrap_or("/ai/v3/refund/"), &token.bearer, &refund_body).await {
            Ok((200, _, text)) => {
                let granted = text.contains("\"granted\":true") || text.contains("\"granted\": true");
                eprintln!("[refund] turn {} — {}", &turn_id[..8.min(turn_id.len())], if granted { "GRANTED (charge reversed)" } else { "denied (quota exhausted)" });
            }
            Ok((s, _, text)) => eprintln!("[refund] turn {} — HTTP {s}: {}", &turn_id[..8.min(turn_id.len())], &text[..text.len().min(120)]),
            Err(e) => eprintln!("[refund] turn {} — transport error: {e}", &turn_id[..8.min(turn_id.len())]),
        }
    }
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
            if tok.usable() {
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
