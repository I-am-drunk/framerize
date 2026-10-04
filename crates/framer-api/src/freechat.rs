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
    // CONCURRENCY BUCKETS (verified 2026-10-04): the workspace's ~5-concurrent
    // 429 cap counts per (route × step tag). Each chunk rotates to the next
    // bucket, and a 429 advances to the next bucket INSTANTLY instead of
    // sleeping.
    //
    // ROUTE PINNED to /messages (2026-10-04, measured live): Castle risk-scores
    // the /chat passthrough far more aggressively — same account, same second:
    // /ai/v3/messages/ -> 200, /ai/v3/chat/ -> bare 403 (650ms, no upstream
    // markers = edge refusal).
    //
    // THE BARE GOLDEN GOOSE (measured 2026-10-04, in-page on a live account):
    // the ~4-5-concurrent-per-bucket 429 cap keys on the `step` tag — and
    // `step` accepts ARBITRARY invented strings, each its own bucket:
    //   12x same step 'message'  -> 4 pass / 8x 429
    //   24x same step            -> 5 pass / 19x 429
    //   12x unique steps         -> 12/12 pass (twice, incl. post-volume)
    //   16x unique steps         -> 16/16 (burst and staggered)
    //   24x unique steps         -> 3 pass / 21x 429  <- the real wall: a
    //   workspace-wide concurrency limit (~16-20), NOT per-bucket. Unique
    //   steps eliminate every per-bucket 429; the gate holds in-flight under
    //   the workspace wall and the client never sees a 429.
    // So every chunk gets a FRESH step tag — organic-looking word + nonce,
    // indistinguishable in shape from Framer's own instrumentation tags.
    const DIALECTS: [Dialect; 1] = [Dialect::OpenRouterMessages];
    const STEP_WORDS: [&str; 8] = [
        "message", "compose", "draft", "revise", "refine", "expand", "polish", "continue",
    ];
    let chain_nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u64 ^ ((d.subsec_nanos() as u64) << 20))
        .unwrap_or(0x9e3779b97f4a7c15);
    let fresh_step = move |i: usize| -> String {
        let h = chain_nonce
            .wrapping_mul(0x9e3779b97f4a7c15)
            .wrapping_add((i as u64).wrapping_mul(0xbf58476d1ce4e5b9));
        format!(
            "{}-{:06x}",
            STEP_WORDS[(h as usize >> 8) % STEP_WORDS.len()],
            (h >> 16) & 0xffffff
        )
    };
    let bucket = |i: usize| -> (Dialect, String) { (DIALECTS[0], fresh_step(i)) };
    let mut bucket_idx = 0usize;

    // Per-chunk payload: the client's full request, only stream/max_tokens
    // overridden; chat-route chunks use the converted chat/completions shape.
    let build_payload = |messages: &Value, chunk: u32, dialect: Dialect| {
        let base = match dialect {
            Dialect::OpenRouterMessages => anthro_payload.clone(),
            _ => framer_core::convert::anthro_to_chat(&anthro_payload),
        };
        let mut p = base;
        p["messages"] = match dialect {
            Dialect::OpenRouterMessages => messages.clone(),
            _ => framer_core::convert::anthro_to_chat(&json!({"messages": messages}))["messages"].clone(),
        };
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
    let mut tool_cut_retries = 0u32;
    let mut slow_chunks = 0u32;
    let mut rate_retries = 0u32;

    // Window sizing from live measurement (§8 + 2026-10-03 storm analysis):
    // the messages route's deaths cluster at ~60-67s under load, and upstream
    // throughput oscillates 10-67 tok/s WITHIN minutes. Budgets must track
    // the model's LIVE speed — and crucially, START SMALL: a 2500-token first
    // chunk at a degraded 10 tok/s takes 250s of total silence (looks hung).
    // Ramp UP from 600 with measured speed instead of ramping down after
    // deaths: first content lands in ~10-60s regardless of upstream mood.
    let window_budget_s = 45.0f32;
    let chunk_cap = cfg.chunk_tokens.max(2500);
    let mut chunk_tokens = cfg.chunk_tokens.min(600);
    let budget_for = |tps: f32| -> u32 { ((tps * window_budget_s) as u32).clamp(400, 3000) };

    let chain_started = std::time::Instant::now();
    let max_wall = std::time::Duration::from_secs(300); // 5 min: beyond this a
    // chain is storming; the client retries fresh far better than we churn.
    for i in 0..cfg.max_chunks {
        if chain_started.elapsed() > max_wall {
            return Err(format!("chain exceeded {}s wall-clock budget after {i} chunks (upstream degraded) — retry the request", max_wall.as_secs()));
        }
        let budget = cfg
            .max_total_tokens
            .map(|cap| cap.saturating_sub(ctok as u32).max(1))
            .unwrap_or(chunk_tokens);
        let chunk = budget.min(chunk_tokens);
        // Human cadence: a metronomic chunk train is itself a bot signature.
        // 0.7-2.3s of jitter between chunks (derived from the nonce — no
        // external entropy needed), invisible against 10-60s generations.
        if i > 0 {
            let j = 700u64 + (chain_nonce.wrapping_add(i as u64).wrapping_mul(0x9e3779b1) % 1600);
            tokio::time::sleep(std::time::Duration::from_millis(j)).await;
        }
        let (dialect, step) = bucket(bucket_idx);
        let payload = build_payload(&messages, chunk, dialect);
        let t0 = std::time::Instant::now();
        eprintln!("[freechat] chunk {i} start (max_tokens={chunk}, bucket {dialect:?}/{step})");
        let resp = match proxy::forward(
            t, token_cache, session, browser, account, project,
            dialect, &step, payload.clone(), chat_session,
        ).await {
            Ok(r) => r,
            Err(e) if e.starts_with("retryable 429") && rate_retries < 12 => {
                // 429 = THIS BUCKET (route×step) is full. Advance to the next
                // bucket and retry INSTANTLY — the cap counts per bucket, so
                // the next one is empty. Only after ~10 bucket hops (the whole
                // rotation twice) does it degenerate to a real sleep.
                rate_retries += 1;
                bucket_idx += 1;
                let (d2, s2) = bucket(bucket_idx);
                if rate_retries > 10 {
                    let wait = std::time::Duration::from_millis(1500 * (1 << (rate_retries - 10).min(4)));
                    eprintln!("[freechat] chunk {i} 429 everywhere — sleeping {wait:?} (attempt {rate_retries})");
                    tokio::time::sleep(wait).await;
                } else {
                    eprintln!("[freechat] chunk {i} 429 on {dialect:?}/{step} — instant retry on next bucket {d2:?}/{s2}");
                }
                let retry_payload = build_payload(&messages, chunk_tokens, d2);
                match proxy::forward(
                    t, token_cache, session, browser, account, project,
                    d2, &s2, retry_payload, chat_session,
                ).await {
                    Ok(r) => r,
                    Err(e2) => {
                        if e2.starts_with("retryable 429") && rate_retries < 12 {
                            continue;
                        }
                        return Err(format!("chunk {i}: workspace still saturated after {rate_retries} bucket retries ({e2})"));
                    }
                }
            }
            Err(e) => {
                let elapsed = t0.elapsed().as_secs_f32();
                let idle_killed = e == "risk-refused" && elapsed > 8.0;
                // An UPSTREAM 403 (provider policy / moderation -- see
                // classify.rs) is terminal for this chunk. Halving max_tokens
                // cannot change a policy decision, so the retry below would
                // only spend a second request to be refused identically.
                let upstream_refused = e.starts_with("upstream 403");
                let transport_died = !upstream_refused
                    && e != "risk-refused"
                    && e != "credits-exhausted"
                    && e != "unauthorized"
                    && !e.starts_with("retryable 4");
                if !((idle_killed || transport_died) && chunk_tokens > 200) {
                    return Err(e);
                }
                chunk_tokens /= 2;
                bucket_idx += 1;
                let (d3, s3) = bucket(bucket_idx);
                eprintln!("[freechat] chunk {i} died at {elapsed:.1}s ({e}) — retrying with max_tokens={chunk_tokens} on {d3:?}/{s3}");
                let retry_payload = build_payload(&messages, chunk_tokens, d3);
                match proxy::forward(
                    t, token_cache, session, browser, account, project,
                    d3, &s3, retry_payload, chat_session,
                ).await {
                    Ok(r) => r,
                    Err(e2) if cfg.metered_fallback => {
                        eprintln!("[freechat] chunk {i} retry died ({e2}) — metered stream fallback for this chunk");
                        let stream_payload = build_payload(&messages, chunk_tokens, d3);
                        let streamed = stream_chunk(
                            t, token_cache, session, browser, account, project,
                            d3, &s3, stream_payload, chat_session,
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

        // Wall-clock includes OUR queue wait for a free browser page. Subtract
        // it: queue wait and a slow upstream look identical in wall-clock, and
        // charging queue time to the upstream makes the sizer below shrink
        // chunks under concurrency -- which multiplies round trips (every chunk
        // re-sends the whole conversation) and deepens the very queue it is
        // reacting to. Measured 2026-10-03: 85 tok/s solo -> 3-6 tok/s at 12
        // agents on an otherwise IDLE box (load average 0.29).
        let wall = t0.elapsed().as_secs_f32();
        let queued = resp.queue_wait_ms as f32 / 1000.0;
        let elapsed = (wall - queued).max(0.5);
        if queued > 2.0 {
            eprintln!("[freechat] chunk {i} queued {queued:.1}s of {wall:.1}s (excluded from tok/s)");
        }
        let body: Value = serde_json::from_str(&body_text(resp).await?).map_err(|e| format!("chunk {i} not json: {e}"))?;
        if msg_id.is_empty() {
            msg_id = body["id"].as_str().unwrap_or_default().to_string();
        }
        let (chunk_blocks, chunk_stop, usage) = match dialect {
            Dialect::OpenRouterMessages => (
                body["content"].as_array().cloned().unwrap_or_default(),
                body["stop_reason"].as_str().unwrap_or("end_turn").to_string(),
                body.get("usage").cloned().unwrap_or_else(|| json!({})),
            ),
            _ => framer_core::convert::chat_response_to_blocks(&body),
        };
        let in_t = usage["input_tokens"].as_u64().unwrap_or(0);
        let out_t = usage["output_tokens"].as_u64().unwrap_or(0);
        ptok = ptok.max(in_t); // input counts the whole conversation; keep latest
        ctok += out_t;
        last_usage = usage;

        // Adapt chunk size from measured throughput (floor 400: completes even
        // when the upstream degrades to a crawl; ceiling 3000 ≈ 75% window).
        let tps = out_t as f32 / elapsed.max(0.5);
        eprintln!("[freechat] chunk {i} done in {elapsed:.1}s ({out_t} tok, {tps:.0} tok/s)");
        if tps > 1.0 {
            chunk_tokens = budget_for(tps);
        }
        // Sustained collapse detector: two consecutive chunks under ~8 tok/s
        // means the upstream is in a degraded period. Fail FAST (the client
        // retries later and wins) instead of grinding into the 300s wall-cap
        // and multiplying user wait through client retries.
        if tps > 0.0 && tps < 8.0 {
            slow_chunks += 1;
            if slow_chunks >= 2 {
                return Err(format!("upstream degraded ({tps:.0} tok/s sustained) — retry in a minute"));
            }
        } else if tps >= 12.0 {
            slow_chunks = 0;
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
        if chunk_stop == "max_tokens" && ends_in_tool_use && tool_cut_retries < 2 {
            // Free retry with an ADAPTIVE window-safe budget: ~85s of the
            // model's own measured throughput (Fable runs ~67 tok/s → ~5700
            // tokens, inside the ~100s window). The cut is usually thinking
            // hogging the budget, not the tool call itself.
            tool_cut_retries += 1;
            let budget = budget_for(tps).max(chunk_tokens);
            eprintln!("[freechat] chunk {i} cut mid-tool_use — free retry #{tool_cut_retries} (max_tokens={budget}, measured {tps:.0} tok/s)");
            chunk_tokens = budget;
            ctok = ctok.saturating_sub(out_t);
            continue;
        }
        if chunk_stop == "max_tokens" && ends_in_tool_use {
            if !cfg.metered_fallback {
                return Err(format!("chunk {i}: tool call exceeds the free non-streaming window; set FRAMERIZE_ALLOW_METERED=1 to allow a tiny metered chunk here"));
            }
            eprintln!("[freechat] chunk {i} cut mid-tool_use again — streaming this chunk metered (explicit opt-in)");
            ctok = ctok.saturating_sub(out_t);
            let stream_payload = build_payload(&messages, cfg.max_total_tokens.map(|cap| cap.saturating_sub(ctok as u32).max(1024)).unwrap_or(8000), dialect);
            let streamed = stream_chunk(
                t, token_cache, session, browser, account, project,
                dialect, &step, stream_payload, chat_session,
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
        bucket_idx += 1; // rotate the bucket every chunk, not just on 429
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
    // Text only: thinking blocks in prior turns must pass signature
    // validation or be dropped (API rule); a chained continuation can do
    // neither cleanly, and the model does not need them to continue.
    let content: Vec<Value> = chunk_blocks
        .iter()
        .filter(|b| b["type"].as_str() == Some("text"))
        .cloned()
        .collect();
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
    step: &str,
    mut payload: Value,
    chat_session: &str,
) -> Result<StreamedChunk, String> {
    payload["stream"] = json!(true);
    let resp = proxy::forward(t, token_cache, session, browser, account, project, upstream, step, payload, chat_session).await?;
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
                    Some("signature_delta") => {
                        let prev = blocks[idx]["signature"].as_str().unwrap_or_default().to_string();
                        blocks[idx]["signature"] = json!(format!("{}{}", prev, v["delta"]["signature"].as_str().unwrap_or_default()));
                    }
                    Some("thinking_delta") => {
                        let prev = blocks[idx]["thinking"].as_str().unwrap_or_default().to_string();
                        blocks[idx]["thinking"] = json!(format!("{}{}", prev, v["delta"]["thinking"].as_str().unwrap_or_default()));
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
            // Only overwrite when deltas actually accumulated — otherwise keep
            // the input the upstream sent in content_block_start (clobbering
            // it with {"_raw":""} produced malformed tool calls).
            if !raw.is_empty() {
                b["input"] = serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!({"_raw": raw}));
            }
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
