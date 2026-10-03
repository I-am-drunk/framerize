//! Unmetered long-completion via continuation chaining on the OpenRouter
//! Anthropic-format passthrough (/ai/v3/messages/), rebuilt BLOCK-AWARE:
//! the client payload passes through with full fidelity (tools, system,
//! tool_choice, …), and the assembled response preserves every content
//! block — text merges, tool_use blocks keep ids/names/inputs, stop_reason
//! and usage are exact.
//!
//! Verified on the live account (verified-captures-2026-10-01.md):
//!   - non-streaming on /ai/v3/messages/ is unmetered; streaming is metered.
//!   - the messages route's idle window is ~60-109s (chat route: ~21.5s).
//! Never-fail ladder per chunk: free call → halving retry (window-death) →
//! metered-stream fallback for that chunk only → chain resumes free.

use std::sync::Mutex;

use serde_json::{json, Value};

use framer_core::convert::{AssembledResponse, ChainEvent};
use framer_core::ids::{AccountId, ProjectId};
use framer_core::port::{BrowserBridge, ChainConfig, Session};

use crate::proxy::{self, Dialect};
use crate::auth::AiToken;
use crate::transport::Transport;
use futures::StreamExt;

/// Chain a long completion. `anthro_payload` is the client's request already
/// converted to Anthropic Messages shape (model normalized); only `stream`
/// and `max_tokens` are overridden per chunk — every other field passes
/// through verbatim on every chunk.
#[allow(clippy::too_many_arguments)]
pub async fn complete_long(
    t: &Transport,
    token_cache: &Mutex<Option<(AccountId, AiToken)>>,
    session: &Session,
    browser: &dyn BrowserBridge,
    account: &AccountId,
    project: &ProjectId,
    anthro_payload: Value,
    cfg: &ChainConfig,
    chat_session: &str,
    // Incremental progress: each chunk's blocks as it completes — the server
    // streams these as SSE frames from the first second (no silent socket).
    events: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
) -> Result<AssembledResponse, String> {
    let upstream = Dialect::OpenRouterMessages;

    // Per-chunk payload: the client's full request, only stream/max_tokens overridden.
    let build_payload = |messages: &Value, chunk: u32| {
        let mut p = anthro_payload.clone();
        p["messages"] = messages.clone();
        p["stream"] = json!(false);
        p["max_tokens"] = json!(chunk);
        p
    };

    let mut messages = anthro_payload["messages"].clone();
    let mut blocks: Vec<Value> = Vec::new();
    let mut msg_id = String::new();
    let mut stop_reason = "end_turn".to_string();
    let mut stop_sequence: Option<String> = None;
    let mut last_usage = json!({});
    let (mut ptok, mut ctok) = (0u64, 0u64);
    let mut metered_chunks = 0u32;

    // Window sizing from live measurement (§8): messages route ~60-109s.
    let window_budget_s = 45.0f32;
    let chunk_cap = cfg.chunk_tokens.max(2500);
    let mut chunk_tokens = chunk_cap;

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
                if !((idle_killed || transport_died) && chunk_tokens > 200) {
                    return Err(e);
                }
                chunk_tokens /= 2;
                eprintln!("[freechat] chunk {i} died at {elapsed:.1}s ({e}) — retrying with max_tokens={chunk_tokens}");
                let retry_payload = build_payload(&messages, chunk_tokens);
                match proxy::forward(
                    t, token_cache, session, browser, account, project,
                    upstream, retry_payload, chat_session,
                ).await {
                    Ok(r) => r,
                    Err(e2) if cfg.metered_fallback => {
                        eprintln!("[freechat] chunk {i} retry died ({e2}) — metered stream fallback for this chunk");
                        let stream_payload = build_payload(&messages, chunk_tokens);
                        let streamed = stream_chunk(
                            t, token_cache, session, browser, account, project,
                            upstream, stream_payload, chat_session,
                        ).await?;
                        metered_chunks += 1;
                        ptok = ptok.max(streamed.input_tokens);
                        ctok += streamed.output_tokens;
                        last_usage = streamed.usage.clone();
                        if msg_id.is_empty() && !streamed.id.is_empty() {
                            msg_id = streamed.id.clone();
                        }
                        let streamed_stop = streamed.stop_reason.clone();
                        let streamed_seq = streamed.stop_sequence.clone();
                        let streamed_blocks = streamed.content.clone();
                        merge_blocks(&mut blocks, streamed_blocks.clone());
                        if let Some(cb) = events {
                            cb(ChainEvent::Chunk(streamed_blocks.clone()));
                        }
                        let finished = streamed_stop != "max_tokens"
                            || cfg.max_total_tokens.map(|cap| cap.saturating_sub(ctok as u32) == 0).unwrap_or(false);
                        if finished {
                            return Ok(assemble(msg_id, blocks, streamed_stop, streamed_seq, last_usage, ptok, ctok, i + 1, metered_chunks));
                        }
                        push_continuation(&mut messages, &streamed_blocks, cfg);
                        continue;
                    }
                    Err(e2) => return Err(format!("chunk {i} retry died (fallback off): {e2}")),
                }
            }
        };

        let elapsed = t0.elapsed().as_secs_f32();
        let body: Value = serde_json::from_str(&body_text(resp).await?).map_err(|e| format!("chunk {i} not json: {e}"))?;
        if msg_id.is_empty() {
            msg_id = body["id"].as_str().unwrap_or_default().to_string();
        }
        let chunk_stop = body["stop_reason"].as_str().unwrap_or("end_turn").to_string();
        let chunk_blocks: Vec<Value> = body["content"].as_array().cloned().unwrap_or_default();
        let usage = body.get("usage").cloned().unwrap_or_else(|| json!({}));
        let in_t = usage["input_tokens"].as_u64().unwrap_or(0);
        let out_t = usage["output_tokens"].as_u64().unwrap_or(0);
        ptok = ptok.max(in_t); // input counts the whole conversation; keep latest
        ctok += out_t;
        last_usage = usage;

        // Adapt chunk size from measured throughput.
        let tps = out_t as f32 / elapsed.max(0.5);
        if tps > 1.0 {
            chunk_tokens = ((tps * window_budget_s) as u32).clamp(150, chunk_cap);
        }

        // Partial tool_use at max_tokens: a half-written tool call can't be
        // continued — and enlarging the budget breaks the real invariant
        // (chunks must finish inside the ~100s idle window; a 5-8k-token tool
        // call at ~20-50 tok/s CANNOT). Enlarging just dies slower (measured:
        // 69s NetworkError deaths, then churn to a client-visible 500). So a
        // mid-tool_use cut routes THIS chunk to metered streaming immediately:
        // bytes flow, no window applies, the tool call completes whole. Tiny
        // credit touch for exactly one chunk; the chain resumes free after.
        let ends_in_tool_use = chunk_blocks
            .last()
            .map(|b| b["type"].as_str() == Some("tool_use"))
            .unwrap_or(false);
        if chunk_stop == "max_tokens" && ends_in_tool_use {
            if !cfg.metered_fallback {
                return Err(format!("chunk {i}: tool call exceeds the free window (fallback off)"));
            }
            eprintln!("[freechat] chunk {i} cut mid-tool_use — streaming this chunk metered (window physics, not enlargement)");
            ctok = ctok.saturating_sub(out_t);
            let stream_payload = build_payload(&messages, cfg.max_total_tokens.map(|cap| cap.saturating_sub(ctok as u32).max(1024)).unwrap_or(8000));
            let streamed = stream_chunk(
                t, token_cache, session, browser, account, project,
                upstream, stream_payload, chat_session,
            ).await?;
            metered_chunks += 1;
            ptok = ptok.max(streamed.input_tokens);
            ctok += streamed.output_tokens;
            last_usage = streamed.usage.clone();
            if msg_id.is_empty() && !streamed.id.is_empty() {
                msg_id = streamed.id.clone();
            }
            let streamed_stop = streamed.stop_reason.clone();
            let streamed_seq = streamed.stop_sequence.clone();
            let streamed_blocks = streamed.content.clone();
            merge_blocks(&mut blocks, streamed_blocks.clone());
            if let Some(cb) = events {
                cb(ChainEvent::Chunk(streamed_blocks.clone()));
            }
            let finished = streamed_stop != "max_tokens"
                || cfg.max_total_tokens.map(|cap| cap.saturating_sub(ctok as u32) == 0).unwrap_or(false);
            if finished {
                return Ok(assemble(msg_id, blocks, streamed_stop, streamed_seq, last_usage, ptok, ctok, i + 1, metered_chunks));
            }
            push_continuation(&mut messages, &streamed_blocks, cfg);
            continue;
        }

        stop_reason = chunk_stop.clone();
        stop_sequence = body["stop_sequence"].as_str().map(str::to_string);
        merge_blocks(&mut blocks, chunk_blocks.clone());
        if let Some(cb) = events {
            cb(ChainEvent::Chunk(chunk_blocks.clone()));
        }

        let budget_left = cfg.max_total_tokens.map(|cap| cap.saturating_sub(ctok as u32));
        if chunk_stop != "max_tokens" || budget_left == Some(0) {
            return Ok(assemble(msg_id, blocks, stop_reason, stop_sequence, last_usage, ptok, ctok, i + 1, metered_chunks));
        }
        push_continuation(&mut messages, &chunk_blocks, cfg);
    }
    Ok(assemble(msg_id, blocks, stop_reason, stop_sequence, last_usage, ptok, ctok, cfg.max_chunks, metered_chunks))
}

async fn body_text(resp: framer_core::port::ProxyResponse) -> Result<String, String> {
    let mut stream = resp.stream;
    let mut buf = bytes::BytesMut::new();
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk.map_err(|e| e.to_string())?);
    }
    String::from_utf8(buf.to_vec()).map_err(|e| e.to_string())
}

fn assemble(
    id: String,
    content: Vec<Value>,
    stop_reason: String,
    stop_sequence: Option<String>,
    last_usage: Value,
    ptok: u64,
    ctok: u64,
    chunks: u32,
    metered_chunks: u32,
) -> AssembledResponse {
    let mut usage = last_usage;
    usage["output_tokens"] = json!(ctok);
    usage["input_tokens"] = json!(ptok);
    AssembledResponse {
        id: if id.is_empty() {
            format!("msg_{}", &uuid::Uuid::new_v4().simple().to_string()[..24])
        } else {
            id
        },
        content,
        stop_reason,
        stop_sequence,
        usage,
        input_tokens: ptok,
        output_tokens: ctok,
        chunks,
        metered_chunks,
    }
}

/// Merge a chunk's content blocks into the accumulator: consecutive text
/// blocks join seamlessly; tool_use and other blocks append verbatim.
fn merge_blocks(acc: &mut Vec<Value>, new: Vec<Value>) {
    for b in new {
        let is_text = b["type"].as_str() == Some("text");
        if is_text {
            if let Some(last) = acc.last_mut() {
                if last["type"].as_str() == Some("text") {
                    let joined = format!(
                        "{}{}",
                        last["text"].as_str().unwrap_or_default(),
                        b["text"].as_str().unwrap_or_default()
                    );
                    last["text"] = json!(joined);
                    continue;
                }
            }
        }
        acc.push(b);
    }
}

/// Append this chunk's blocks as the assistant turn + the continue prompt.
/// Only ever called when the chunk is text-only (any tool_use ends the
/// chain), so the assistant message is spec-valid.
fn push_continuation(messages: &mut Value, chunk_blocks: &[Value], cfg: &ChainConfig) {
    let content: Vec<Value> = chunk_blocks.to_vec();
    let arr = messages.as_array_mut().expect("messages array");
    arr.push(json!({"role": "assistant", "content": content}));
    arr.push(json!({"role": "user", "content": cfg.continue_prompt}));
}

// ---------------------------------------------------------------------------
// Metered streaming fallback (rung 3): consume one SSE stream, BLOCK-AWARE.
// ---------------------------------------------------------------------------

struct StreamedChunk {
    id: String,
    content: Vec<Value>,
    stop_reason: String,
    stop_sequence: Option<String>,
    usage: Value,
    input_tokens: u64,
    output_tokens: u64,
}

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
) -> Result<StreamedChunk, String> {
    payload["stream"] = json!(true);
    let resp = proxy::forward(t, token_cache, session, browser, account, project, upstream, payload, chat_session).await?;
    let text = body_text(resp).await?;

    let mut id = String::new();
    let mut blocks: Vec<Value> = Vec::new();
    let mut stop_reason = "end_turn".to_string();
    let mut stop_sequence: Option<String> = None;
    let (mut ptok, mut ctok) = (0u64, 0u64);
    let mut current_data = String::new();

    fn handle_event(
        data: &str,
        id: &mut String,
        blocks: &mut Vec<Value>,
        stop_reason: &mut String,
        stop_sequence: &mut Option<String>,
        ptok: &mut u64,
        ctok: &mut u64,
    ) {
        let Ok(v) = serde_json::from_str::<Value>(data) else { return };
        match v["type"].as_str() {
            Some("message_start") => {
                *id = v["message"]["id"].as_str().unwrap_or_default().to_string();
                *ptok = v["message"]["usage"]["input_tokens"].as_u64().unwrap_or(0);
            }
            Some("content_block_start") => {
                let idx = v["index"].as_u64().unwrap_or(blocks.len() as u64) as usize;
                while blocks.len() <= idx {
                    blocks.push(json!({"type":"text","text":""}));
                }
                blocks[idx] = v["content_block"].clone();
            }
            Some("content_block_delta") => {
                let idx = v["index"].as_u64().unwrap_or(0) as usize;
                while blocks.len() <= idx {
                    blocks.push(json!({"type":"text","text":""}));
                }
                match v["delta"]["type"].as_str() {
                    Some("text_delta") => {
                        let joined = format!(
                            "{}{}",
                            blocks[idx]["text"].as_str().unwrap_or_default(),
                            v["delta"]["text"].as_str().unwrap_or_default()
                        );
                        blocks[idx]["text"] = json!(joined);
                    }
                    Some("input_json_delta") => {
                        let prev = blocks[idx]["_partial_json"].as_str().unwrap_or_default().to_string();
                        blocks[idx]["_partial_json"] =
                            json!(format!("{}{}", prev, v["delta"]["partial_json"].as_str().unwrap_or_default()));
                    }
                    _ => {}
                }
            }
            Some("message_delta") => {
                *stop_reason = v["delta"]["stop_reason"].as_str().unwrap_or("end_turn").to_string();
                *stop_sequence = v["delta"]["stop_sequence"].as_str().map(str::to_string);
                *ctok = v["usage"]["output_tokens"].as_u64().unwrap_or(*ctok);
            }
            _ => {}
        }
    }

    for line in text.lines() {
        if let Some(d) = line.strip_prefix("data:") {
            let d = d.trim_start();
            if !current_data.is_empty() {
                current_data.push('\n');
            }
            current_data.push_str(d);
        } else if line.trim().is_empty() && !current_data.is_empty() {
            handle_event(&current_data, &mut id, &mut blocks, &mut stop_reason, &mut stop_sequence, &mut ptok, &mut ctok);
            current_data.clear();
        }
    }
    if !current_data.is_empty() {
        handle_event(&current_data, &mut id, &mut blocks, &mut stop_reason, &mut stop_sequence, &mut ptok, &mut ctok);
    }

    // finalize tool_use blocks: parse the accumulated partial JSON into `input`
    for b in blocks.iter_mut() {
        if b["type"].as_str() == Some("tool_use") {
            let raw = b["_partial_json"].as_str().unwrap_or_default().to_string();
            let parsed = serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!({"_raw": raw}));
            b["input"] = parsed;
            if let Some(obj) = b.as_object_mut() {
                obj.remove("_partial_json");
            }
        }
    }

    Ok(StreamedChunk {
        id,
        content: blocks,
        stop_reason,
        stop_sequence,
        usage: json!({"input_tokens": ptok, "output_tokens": ctok}),
        input_tokens: ptok,
        output_tokens: ctok,
    })
}
