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
}

impl Dialect {
    pub fn path(self) -> &'static str {
        match self {
            Self::OpenAiResponses => "/ai/v3/openai/responses/",
            Self::OpenRouterChat => "/ai/v3/chat/",
            Self::AnthropicMessages => "/ai/v3/anthropic/messages/",
            Self::OpenRouterMessages => "/ai/v3/messages/",
        }
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
) -> Result<framer_core::port::ProxyResponse, String> {
    let t0 = std::time::Instant::now();
    let req_id = uuid::Uuid::new_v4().to_string();
    let token = cached_token(t, token_cache, session, account).await?;
    let category = std::env::var("FRAMERIZE_FEATURE_CATEGORY")
        .unwrap_or_else(|_| "translation".to_string());
    match attempt(browser, session, account, &token, project, dialect, step, &payload, chat_session, &category).await {
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
            attempt(browser, session, account, &fresh, project, dialect, step, &payload, chat_session, &category).await
        }
        Err(e) if e == "category-rejected" => {
            eprintln!("[proxy] model rejected for translation category — retrying as agents");
            attempt(browser, session, account, &token, project, dialect, step, &payload, chat_session, "agents").await
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
    let turn_id = crate::inference::mint_id_pub();
    let body = json!({
        "projectId": project.0,
        "featureCategory": category,
        "featureName": "agents",
        "step": step,
        "sessionId": chat_session,
        "stepIndex": 0,
        "turnId": turn_id,
        "userPrompt": "", // proxy calls carry their content in `request`
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
            "featureName": "agents",
            "step": step,
            "sessionId": chat_session,
            "stepIndex": 0,
            "turnId": turn_id,
            "userPrompt": "",
            "castleToken": "garbage", // route ignores it (verified)
        });
        match browser.ai_post(account, "/ai/v3/refund/", &token.bearer, &refund_body).await {
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
