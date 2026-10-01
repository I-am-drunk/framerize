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
    system: Option<String>,
    messages: Vec<Value>,
    cfg: &framer_core::port::ChainConfig,
    chat_session: &str,
    dialect: framer_core::port::ChainDialect,
) -> Result<ChainResult, String> {
    let upstream = match dialect {
        framer_core::port::ChainDialect::OpenAiChat => Dialect::OpenRouterChat,
        framer_core::port::ChainDialect::AnthropicMessages => Dialect::OpenRouterMessages,
    };
    let build_payload = |messages: &Vec<Value>, chunk: u32| {
        let mut p = json!({"model": model, "messages": messages, "max_tokens": chunk, "stream": false});
        if let (framer_core::port::ChainDialect::AnthropicMessages, Some(sys)) = (dialect, &system) {
            p["system"] = json!(sys);
        }
        p
    };
    // -> (piece_text, is_length_finish, input_tokens, output_tokens)
    let extract = |body: &Value, i: u32| -> Result<(String, bool, u64, u64), String> {
        match dialect {
            framer_core::port::ChainDialect::OpenAiChat => {
                let choice = body.get("choices").and_then(|c| c.get(0))
                    .ok_or_else(|| format!("chunk {i}: no choices: {}", &body.to_string()[..200.min(body.to_string().len())]))?;
                let piece = choice.pointer("/message/content").and_then(Value::as_str).unwrap_or_default().to_string();
                let finish = choice.get("finish_reason").and_then(Value::as_str).unwrap_or("stop");
                let u = body.get("usage");
                Ok((piece, finish == "length",
                    u.and_then(|u| u.get("prompt_tokens")).and_then(Value::as_u64).unwrap_or(0),
                    u.and_then(|u| u.get("completion_tokens")).and_then(Value::as_u64).unwrap_or(0)))
            }
            framer_core::port::ChainDialect::AnthropicMessages => {
                let piece: String = body.get("content").and_then(Value::as_array)
                    .map(|blocks| blocks.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join(""))
                    .unwrap_or_default();
                let stop = body.get("stop_reason").and_then(Value::as_str).unwrap_or("end_turn");
                let u = body.get("usage");
                Ok((piece, stop == "max_tokens",
                    u.and_then(|u| u.get("input_tokens")).and_then(Value::as_u64).unwrap_or(0),
                    u.and_then(|u| u.get("output_tokens")).and_then(Value::as_u64).unwrap_or(0)))
            }
        }
    };
    let mut messages = messages;
    let mut text = String::new();
    let (mut ptok, mut ctok) = (0u64, 0u64);
    // Adaptive window safety: the upstream kills connections idle >~21.5s.
    // Measure the model's live tokens/sec from each completed chunk and size
    // the next one to finish inside ~12s (≈60% of the window) — slow models
    // shrink, fast models stretch. A chunk that DIES (transport reset, or a
    // "risk-refused" 403 that only arrives after a long silence — the idle
    // killer's signature, distinct from Castle's instant refusals) halves the
    // size and retries once.
    let mut chunk_tokens = cfg.chunk_tokens;
    const WINDOW_BUDGET_S: f32 = 12.0;

    for i in 0..cfg.max_chunks {
        let budget = cfg
            .max_total_tokens
            .map(|cap| cap.saturating_sub(ctok as u32).max(1))
            .unwrap_or(chunk_tokens);
        let chunk = budget.min(chunk_tokens);
        let payload = build_payload(&messages, chunk);
        let t0 = std::time::Instant::now();
        let resp = match proxy::forward(
            t, token_cache, session, browser, account, project,
            upstream, payload.clone(), chat_session,
        ).await {
            Ok(r) => r,
            Err(e) => {
                let elapsed = t0.elapsed().as_secs_f32();
                let idle_killed = e == "risk-refused" && elapsed > 8.0;
                let transport_died = e != "risk-refused"
                    && e != "credits-exhausted"
                    && e != "unauthorized"
                    && !e.starts_with("retryable 4");
                if (idle_killed || transport_died) && chunk_tokens > 200 {
                    chunk_tokens /= 2;
                    eprintln!(
                        "[freechat] chunk {i} died at {elapsed:.1}s ({e}) — retrying with max_tokens={chunk_tokens}"
                    );
                    let retry_payload = build_payload(&messages, chunk_tokens);
                    let t1 = std::time::Instant::now();
                    let r = proxy::forward(
                        t, token_cache, session, browser, account, project,
                        upstream, retry_payload, chat_session,
                    ).await?;
                    let el = t1.elapsed().as_secs_f32();
                    if let Ok(v) = r.json::<Value>().await {
                        let (piece, is_length, pt, ct) = extract(&v, i)?;
                        let tps = ct as f32 / el.max(0.5);
                        if tps > 1.0 {
                            chunk_tokens = ((tps * WINDOW_BUDGET_S) as u32)
                                .clamp(150, cfg.chunk_tokens);
                        }
                        ptok += pt;
                        ctok += ct;
                        text.push_str(&piece);
                        let budget_left = cfg.max_total_tokens.map(|cap| cap.saturating_sub(ctok as u32));
                        if !is_length || budget_left == Some(0) {
                            return Ok(ChainResult { text, chunks: i + 1, prompt_tokens: ptok, completion_tokens: ctok, finish_reason: if is_length { "length".into() } else { "stop".into() } });
                        }
                        messages.push(json!({"role": "assistant", "content": piece}));
                        messages.push(json!({"role": "user", "content": cfg.continue_prompt}));
                        continue;
                    }
                    return Err(format!("chunk {i} retry: bad json"));
                }
                return Err(e);
            }
        };
        // success path: adapt chunk size from measured throughput
        let elapsed = t0.elapsed().as_secs_f32();
        let body: Value = resp.json().await.map_err(|e| format!("chunk {i} not json: {e}"))?;
        let (piece, is_length, pt, ct) = extract(&body, i)?;
        ptok += pt;
        ctok += ct;
        let tps = ct as f32 / elapsed.max(0.5);
        if tps > 1.0 {
            chunk_tokens = ((tps * WINDOW_BUDGET_S) as u32).clamp(150, cfg.chunk_tokens);
        }
        text.push_str(&piece);
        let budget_left = cfg.max_total_tokens.map(|cap| cap.saturating_sub(ctok as u32));
        if !is_length || budget_left == Some(0) {
            return Ok(ChainResult { text, chunks: i + 1, prompt_tokens: ptok, completion_tokens: ctok, finish_reason: if is_length { "length".into() } else { "stop".into() } });
        }
        messages.push(json!({"role": "assistant", "content": piece}));
        messages.push(json!({"role": "user", "content": cfg.continue_prompt}));
    }
    Ok(ChainResult { text, chunks: cfg.max_chunks, prompt_tokens: ptok, completion_tokens: ctok, finish_reason: "length(chunk_cap)".into() })
}
