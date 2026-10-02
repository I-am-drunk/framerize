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
use futures::StreamExt;
use crate::auth::AiToken;
use framer_core::ids::{AccountId, ProjectId};
use framer_core::port::{BrowserBridge, ChainResult, Session};
use serde_json::{json, Value};
use std::sync::Mutex;

/// The never-fail rung: run ONE chunk as a metered SSE stream (bytes flow →
/// the idle-kill can never fire). Consumes anthropic (`content_block_delta`)
/// and chat (`choices[0].delta.content`) SSE shapes; returns (text, in, out).
#[allow(clippy::too_many_arguments)]
async fn stream_chunk(
    t: &Transport,
    token_cache: &Mutex<Option<(AccountId, AiToken)>>,
    session: &Session,
    browser: &dyn BrowserBridge,
    account: &AccountId,
    project: &ProjectId,
    upstream: Dialect,
    mut payload: Value,
    chat_session: &str,
) -> Result<(String, u64, u64), String> {
    payload["stream"] = json!(true);
    let resp = proxy::forward(
        t, token_cache, session, browser, account, project,
        upstream, payload, chat_session,
    ).await?;
    let mut stream = resp.stream;
    let mut buf = String::new();
    let mut text = String::new();
    let (mut ptok, mut ctok) = (0u64, 0u64);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("stream read: {e}"))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find("\n\n") {
            let frame = buf[..pos].to_string();
            buf = buf[pos + 2..].to_string();
            for line in frame.lines() {
                let Some(data) = line.strip_prefix("data:") else { continue };
                let data = data.trim();
                if data == "[DONE]" { break; }
                let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
                // anthropic: content_block_delta → delta.text
                if v.get("type").and_then(Value::as_str) == Some("content_block_delta") {
                    if let Some(t) = v.pointer("/delta/text").and_then(Value::as_str) {
                        text.push_str(t);
                    }
                }
                // anthropic usage: message_start (input) / message_delta (output)
                if let Some(u) = v.pointer("/message/usage/input_tokens").and_then(Value::as_u64) { ptok = u; }
                if let Some(u) = v.pointer("/usage/output_tokens").and_then(Value::as_u64) { ctok = u; }
                // openai chat: choices[0].delta.content + final usage
                if let Some(t) = v.pointer("/choices/0/delta/content").and_then(Value::as_str) {
                    text.push_str(t);
                }
                if let Some(u) = v.pointer("/usage/prompt_tokens").and_then(Value::as_u64) { ptok = u; }
                if let Some(u) = v.pointer("/usage/completion_tokens").and_then(Value::as_u64) { ctok = u; }
            }
        }
    }
    Ok((text, ptok, ctok))
}

/// Canonical-logging wrapper: every chain exits through exactly one
/// `ai.chain` event (see emit_chain). Public entry point.
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
    let t0 = std::time::Instant::now();
    let req_id = uuid::Uuid::new_v4().to_string();
    let r = complete_long_inner(
        t, token_cache, session, browser, account, project, model, system, messages, cfg,
        chat_session, dialect,
    )
    .await;
    emit_chain(t0, &req_id, account, model, dialect, &r);
    r
}

/// Drain a ProxyResponse body into a String (in-page transport buffers the
/// upstream body whole — one chunk — but keep this generic).
async fn body_text(resp: framer_core::port::ProxyResponse) -> Result<String, String> {
    let mut buf = Vec::new();
    let mut s = resp.stream;
    while let Some(c) = s.next().await {
        buf.extend_from_slice(&c?);
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

/// Canonical one-line-per-chain event (docs/logging.md): chunks, metered
/// chunks (the credit-touching count), tokens, wall time, outcome.
fn emit_chain(
    t0: std::time::Instant,
    req_id: &str,
    account: &AccountId,
    model: &str,
    dialect: framer_core::port::ChainDialect,
    r: &Result<framer_core::port::ChainResult, String>,
) {
    use framer_core::observability::{elapsed_ms, Event};
    let mut ev = Event::new("ai.chain")
        .request_id(req_id)
        .account(account)
        .field("model", model.to_string())
        .field("dialect", format!("{dialect:?}"))
        .duration_ms(elapsed_ms(t0));
    ev = match r {
        Ok(res) => ev
            .field("chunks", res.chunks)
            .field("metered_chunks", res.metered_chunks)
            .field("prompt_tokens", res.prompt_tokens)
            .field("completion_tokens", res.completion_tokens)
            .field("finish_reason", res.finish_reason.clone())
            .outcome("ok"),
        Err(e) => ev.error(e),
    };
    ev.emit();
}

/// Run a chat/completions request as a chain of free non-streaming calls.
/// `messages` is the OpenAI-style messages array; `model` a PREFIXED OpenRouter
/// catalog id (e.g. "openai/gpt-4.1", "anthropic/claude-opus-5.5").
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
async fn complete_long_inner(
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
    let mut metered_chunks = 0u32;
    // Adaptive window safety: the upstream kills connections idle >~21.5s.
    // Measure the model's live tokens/sec from each completed chunk and size
    // the next one to finish inside ~12s (≈60% of the window) — slow models
    // shrink, fast models stretch. A chunk that DIES (transport reset, or a
    // "risk-refused" 403 that only arrives after a long silence — the idle
    // killer's signature, distinct from Castle's instant refusals) halves the
    // size and retries once.
    // Window budgets from live measurement (verified-captures §8): the chat
    // route's upstream idle-kills at ~21.5s; the messages route survived 44.9s+
    // (its window is the anthropic-path ~60s config). Size chunks to ~60% of
    // the route's window, and let the messages route use far bigger chunks —
    // most real generations fit in ONE free call there, no chaining at all.
    let (window_budget_s, chunk_cap) = match dialect {
        framer_core::port::ChainDialect::OpenAiChat => (12.0, cfg.chunk_tokens),
        framer_core::port::ChainDialect::AnthropicMessages => (45.0, cfg.chunk_tokens.max(2500)),
    };
    let mut chunk_tokens = match dialect {
        framer_core::port::ChainDialect::OpenAiChat => cfg.chunk_tokens,
        framer_core::port::ChainDialect::AnthropicMessages => chunk_cap,
    };

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
                    let r = match proxy::forward(
                        t, token_cache, session, browser, account, project,
                        upstream, retry_payload, chat_session,
                    ).await {
                        Ok(r) => r,
                        Err(e2) if cfg.metered_fallback => {
                            // Second death on the same chunk — the window or a
                            // provider stall is winning. Metered streaming
                            // cannot idle-die (bytes flow): finish THIS chunk
                            // streamed, then resume the free chain. This is the
                            // rung that makes a long request NEVER fail.
                            eprintln!("[freechat] chunk {i} retry died ({e2}) — metered stream fallback for this chunk");
                            let stream_payload = build_payload(&messages, chunk_tokens);
                            let (piece, pt, ct) = stream_chunk(
                                t, token_cache, session, browser, account, project,
                                upstream, stream_payload, chat_session,
                            ).await?;
                            metered_chunks += 1;
                            ptok += pt;
                            ctok += ct;
                            text.push_str(&piece);
                            let budget_left = cfg.max_total_tokens.map(|cap| cap.saturating_sub(ctok as u32));
                            // streamed chunk ran to its natural stop or filled
                            // the chunk budget — either way the chain continues
                            // if budget remains and the piece filled the chunk.
                            if budget_left == Some(0) || piece.is_empty() {
                                return Ok(ChainResult { text, chunks: i + 1, prompt_tokens: ptok, completion_tokens: ctok, finish_reason: "stop".into(), metered_chunks });
                            }
                            messages.push(json!({"role": "assistant", "content": piece}));
                            messages.push(json!({"role": "user", "content": cfg.continue_prompt}));
                            continue;
                        }
                        Err(e2) => return Err(format!("chunk {i} retry died (fallback off): {e2}")),
                    };
                    let el = t1.elapsed().as_secs_f32();
                    if let Ok(v) = body_text(r).await.and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string())) {
                        let (piece, is_length, pt, ct) = extract(&v, i)?;
                        let tps = ct as f32 / el.max(0.5);
                        if tps > 1.0 {
                            chunk_tokens = ((tps * window_budget_s) as u32)
                                .clamp(150, cfg.chunk_tokens);
                        }
                        ptok += pt;
                        ctok += ct;
                        text.push_str(&piece);
                        let budget_left = cfg.max_total_tokens.map(|cap| cap.saturating_sub(ctok as u32));
                        if !is_length || budget_left == Some(0) {
                            return Ok(ChainResult { text, chunks: i + 1, prompt_tokens: ptok, completion_tokens: ctok, finish_reason: if is_length { "length".into() } else { "stop".into() }, metered_chunks });
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
        let body: Value = serde_json::from_str(&body_text(resp).await?).map_err(|e| format!("chunk {i} not json: {e}"))?;
        let (piece, is_length, pt, ct) = extract(&body, i)?;
        ptok += pt;
        ctok += ct;
        let tps = ct as f32 / elapsed.max(0.5);
        if tps > 1.0 {
            chunk_tokens = ((tps * window_budget_s) as u32).clamp(150, cfg.chunk_tokens);
        }
        text.push_str(&piece);
        let budget_left = cfg.max_total_tokens.map(|cap| cap.saturating_sub(ctok as u32));
        if !is_length || budget_left == Some(0) {
            return Ok(ChainResult { text, chunks: i + 1, prompt_tokens: ptok, completion_tokens: ctok, finish_reason: if is_length { "length".into() } else { "stop".into() }, metered_chunks });
        }
        messages.push(json!({"role": "assistant", "content": piece}));
        messages.push(json!({"role": "user", "content": cfg.continue_prompt}));
    }
    Ok(ChainResult { text, chunks: cfg.max_chunks, prompt_tokens: ptok, completion_tokens: ctok, finish_reason: "length(chunk_cap)".into(), metered_chunks })
}
