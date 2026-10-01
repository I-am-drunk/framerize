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
use crate::castle;
use crate::transport::{Auth, Transport};
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

/// Forward one native payload through the gateway. Returns the raw upstream
/// response (SSE stream or JSON) for the caller to pass through. One 401
/// retry with a fresh JWT, mirroring inference.rs.
#[allow(clippy::too_many_arguments)]
pub async fn forward(
    t: &Transport,
    token_cache: &Mutex<Option<(AccountId, AiToken)>>,
    session: &Session,
    browser: &dyn BrowserBridge,
    account: &AccountId,
    project: &ProjectId,
    dialect: Dialect,
    payload: Value,
    chat_session: &str,
) -> Result<reqwest::Response, String> {
    let token = cached_token(t, token_cache, session, account).await?;
    let castle = castle::mint(browser, account).await?;
    match attempt(t, &token, &castle.0, project, dialect, &payload, chat_session).await? {
        Ok(resp) => Ok(resp),
        Err(e) if e == "unauthorized" => {
            if let Ok(mut guard) = token_cache.lock() {
                *guard = None;
            }
            let fresh = cached_token(t, token_cache, session, account).await?;
            let castle = castle::mint(browser, account).await?;
            match attempt(t, &fresh, &castle.0, project, dialect, &payload, chat_session).await? {
                Ok(resp) => Ok(resp),
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    }
}

type Attempt = Result<reqwest::Response, String>;

async fn attempt(
    t: &Transport,
    token: &AiToken,
    castle: &str,
    project: &ProjectId,
    dialect: Dialect,
    payload: &Value,
    chat_session: &str,
) -> Result<Attempt, String> {
    let body = json!({
        "projectId": project.0,
        "featureCategory": "agents",
        "featureName": "agents",
        "step": "message",
        "sessionId": chat_session,
        "stepIndex": 0,
        "turnId": crate::inference::mint_id_pub(),
        "userPrompt": "", // proxy calls carry their content in `request`
        "castleToken": castle,
        "request": payload,
    });
    match t.post(dialect.path(), Auth::Bearer(&token.bearer), body).await {
        Ok(resp) => Ok(Ok(resp)),
        Err(e) if e == "unauthorized" => Ok(Err(e)),
        Err(e) => Err(e),
    }
}

async fn cached_token(
    t: &Transport,
    cache: &Mutex<Option<(AccountId, AiToken)>>,
    session: &Session,
    account: &AccountId,
) -> Result<AiToken, String> {
    if let Ok(guard) = cache.lock() {
        if let Some((id, tok)) = guard.as_ref() {
            if id == account && tok.usable() {
                return Ok(tok.clone());
            }
        }
    }
    let tok = auth::ai_access_token(t, session).await?;
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((*account, tok.clone()));
    }
    Ok(tok)
}
