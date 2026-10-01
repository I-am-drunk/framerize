//! Free long-form generation on the OpenRouter chat route: continuation chaining.
//!
//! Verified live 2026-10-01 (reference/analysis/verified-captures-2026-10-01.md §6-7):
//! - `/ai/v3/chat/` with `stream:false` is UNMETERED (zero ledger debit, ever);
//!   `stream:true` is metered (303.03 credits per $1 upstream, posts in ~1 min).
//! - BUT a single non-streaming call dies when the upstream holds the connection
//!   with zero bytes too long: observed connection resets at ~21.5 s (twice,
//!   identical) and a rarer generic-403 kill at ~12 s. (Control experiment: a
//!   35 s idle hold to an unrelated host through the same network path also
//!   died, so the cutoff is an idle-connection kill on the path — the fix below
//!   works regardless of which layer owns it, including a server-side one.)
//!
//! The workaround: chunk the generation. Request `max_tokens = chunk_tokens`
//! with `stream:false`; each call finishes well under the idle-kill threshold
//! (600 tokens ≈ 6-10 s on gpt-4.1). When `finish_reason == "length"`, append
//! the partial assistant message and a bare "continue" user message, and call
//! again. Verified live: 4 chunks x 600 tokens = 1,893 words continuous story,
//! all 200s, zero credits debited.

use crate::proxy::{self, Dialect};
use crate::transport::Transport;
use crate::auth::AiToken;
use framer_core::ids::{AccountId, ProjectId};
use framer_core::port::{BrowserBridge, ChainResult, Session};
use serde_json::{json, Value};
use std::sync::Mutex;

/// Run a chat/completions request as a chain of free non-streaming calls.
/// `messages` is the OpenAI-style messages array; `model` a PREFIXED OpenRouter
/// catalog id (e.g. "openai/gpt-4.1", "anthropic/claude-opus-5.5").
#[allow(clippy::too_many_arguments)]
pub async fn complete_long(
    t: &Transport,
    token_cache: &Mutex<Option<(AccountId, AiToken)>>,
    session: &Session,
    browser: &dyn BrowserBridge,
    account: &AccountId,
    project: &ProjectId,
    model: &str,
    messages: Vec<Value>,
    cfg: &framer_core::port::ChainConfig,
    chat_session: &str,
) -> Result<ChainResult, String> {
    let mut messages = messages;
    let mut text = String::new();
    let (mut ptok, mut ctok) = (0u64, 0u64);

    for i in 0..cfg.max_chunks {
        let budget = cfg
            .max_total_tokens
            .map(|cap| cap.saturating_sub(ctok as u32).max(1))
            .unwrap_or(cfg.chunk_tokens);
        let chunk = budget.min(cfg.chunk_tokens);
        let payload = json!({
            "model": model,
            "messages": messages,
            "max_tokens": chunk,
            "stream": false,
        });
        let resp = proxy::forward(
            t, token_cache, session, browser, account, project,
            Dialect::OpenRouterChat, payload, chat_session,
        ).await?;
        let body: Value = resp.json().await.map_err(|e| format!("chunk {i} not json: {e}"))?;
        let choice = body
            .get("choices").and_then(|c| c.get(0))
            .ok_or_else(|| format!("chunk {i}: no choices: {}", &body.to_string()[..200.min(body.to_string().len())]))?;
        let piece = choice
            .pointer("/message/content").and_then(Value::as_str).unwrap_or_default().to_string();
        let finish = choice.get("finish_reason").and_then(Value::as_str).unwrap_or("stop").to_string();
        if let Some(u) = body.get("usage") {
            ptok += u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0);
            ctok += u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0);
        }
        text.push_str(&piece);
        let budget_left = cfg.max_total_tokens.map(|cap| cap.saturating_sub(ctok as u32));
        if finish != "length" || budget_left == Some(0) {
            return Ok(ChainResult { text, chunks: i + 1, prompt_tokens: ptok, completion_tokens: ctok, finish_reason: finish });
        }
        messages.push(json!({"role": "assistant", "content": piece}));
        messages.push(json!({"role": "user", "content": cfg.continue_prompt}));
    }
    Ok(ChainResult { text, chunks: cfg.max_chunks, prompt_tokens: ptok, completion_tokens: ctok, finish_reason: "length(chunk_cap)".into() })
}
