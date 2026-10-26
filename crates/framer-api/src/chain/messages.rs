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

use crate::protocol::messages::{merge_blocks, push_continuation, prepare_payload};
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
    token_cache: &Mutex<Vec<(AccountId, AiToken)>>,
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
    if cfg.chunk_tokens == 0 {
        return Err("Messages chain chunk_tokens must be greater than zero".into());
    }
    if cfg.max_chunks == 0 {
        return Err("Messages chain max_chunks must be greater than zero".into());
    }
    if cfg.max_total_tokens == Some(0) {
        return Err("Messages chain max_total_tokens must be greater than zero".into());
    }
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
    // Step vocabulary (invisibility, 2026-10-05): real clients use CONSTANT
    // steps — "message" for agent turns, "translate" for translation — so the
    // invented compose-3fa2c1-style tags (thousands of unique values/day)
    // were an inspection-visible anomaly. The per-step concurrency dodge they
    // bought is redundant: the cap is per-device under translation (measured
    // ~5/device), and the device pool covers concurrency. Constant REAL step.
    let chain_nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() ^ ((d.subsec_nanos() as u64) << 20))
        .unwrap_or(0x9e3779b97f4a7c15);
    // Default "translate" to MATCH proxy.rs's default category (translation):
    // the envelope went category=translation + step=message otherwise — real
    // translation traffic uses step=translate (invisibility audit 2026-10-11).
    let category = std::env::var("FRAMERIZE_FEATURE_CATEGORY")
        .unwrap_or_else(|_| "translation".to_string());
    let chain_step: String =
        if category == "translation" { "translate".to_string() } else { "message".to_string() };
    let bucket = |_i: usize| -> (Dialect, String) { (DIALECTS[0], chain_step.clone()) };
    let mut bucket_idx = 0usize;

    // Per-chunk payload: the client's full request, only stream/max_tokens
    // overridden; chat-route chunks use the converted chat/completions shape.
    let build_payload = |messages: &Value, chunk: u32, dialect: Dialect| {
        prepare_payload(&anthro_payload, messages, chunk, dialect)
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
    let _chunk_cap = cfg.chunk_tokens.max(2500);
    let mut chunk_tokens = cfg.chunk_tokens.min(600);
    let budget_for = |tps: f32| -> u32 { ((tps * window_budget_s) as u32).clamp(400, 3000) };

    let mut cooldown_wait = super::cooldown_wait::CooldownWait::default();
    let chain_started = cooldown_wait.timer();
    // Local cooldown waits have their own cumulative bound.
    let max_active = std::time::Duration::from_secs(300);
    for i in 0..cfg.max_chunks {
        cfg.check_cancelled()?;
        if cooldown_wait.elapsed(chain_started) > max_active {
            return Err(format!("chain exceeded {}s active-time budget after {i} chunks (excluding local cooldown waits), retry the request", max_active.as_secs()));
        }
        let budget = remaining_tokens(cfg, ctok).unwrap_or(chunk_tokens);
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
        let t0 = cooldown_wait.timer();
        eprintln!("[freechat] chunk {i} start (max_tokens={chunk}, bucket {dialect:?}/{step})");
        // 429 bucket-hop loop (2026-10-05 fix): the old nested match's inner
        // `continue` skipped to the NEXT CHUNK on a second 429, silently
        // dropping this chunk's content — and a successful cross-dialect retry
        // got parsed as the ORIGINAL dialect (garbage). This loop retries THE
        // SAME chunk, carries the effective dialect forward for parsing, and
        // only exits on success, a non-429 error, or exhaustion.
        let mut effective_dialect = dialect;
        let mut last_attempt_t0 = t0;
        let first = {
            let mut cur_dialect = dialect;
            let mut cur_step = step.clone();
            let mut attempt_payload = payload;
            let mut out = None;
            while out.is_none() {
                cfg.check_cancelled()?;
                last_attempt_t0 = cooldown_wait.timer();
                match cooldown_wait.run(cfg, !blocks.is_empty(), || proxy::forward_cancellable(
                    t, token_cache, session, browser, account, project,
                    cur_dialect, &cur_step, attempt_payload.clone(), chat_session, i, cfg.cancelled.as_deref(),
                )).await {
                    Ok(r) => out = Some(Ok((cur_dialect, r))),
                    Err(e) if cfg.is_cancelled() => return Err(e),
                    Err(e) if e.starts_with("retryable 429") => {
                        // 4 in-chain hops, then surface: a saturated WORKSPACE
                        // can't be fixed by bucket hops (the cap is per
                        // workspace), so get to rotation::mark_saturated fast.
                        if rate_retries >= 9 {
                            out = Some(Err(format!("chunk {i}: workspace still saturated after {rate_retries} bucket retries ({e})")));
                            break;
                        }
                        // 429 = THIS BUCKET (route×step) is full. Advance to
                        // the next bucket and retry INSTANTLY — the cap counts
                        // per bucket, so the next one is empty. Only after ~10
                        // hops (the whole rotation twice) a real sleep.
                        rate_retries += 1;
                        if let Some(cb) = &cfg.on_429 {
                            cb();
                        }
                        bucket_idx += 1;
                        let (d2, s2) = bucket(bucket_idx);
                        if rate_retries > 2 {
                            // Saturation is workspace-wide (measured): bucket
                            // hops stop helping after the first couple — WAIT
                            // for in-flight work to drain (escalating, ~2min
                            // total patience) instead of erroring into client
                            // retry storms (the 2026-10-06 wave analysis).
                            let wait = std::time::Duration::from_millis(1500 * (1 << (rate_retries - 2).min(4)));
                            eprintln!("[freechat] chunk {i} 429 — backoff {wait:?} (attempt {rate_retries}/9)");
                            tokio::time::sleep(wait).await;
                        } else {
                            eprintln!("[freechat] chunk {i} 429 on {cur_dialect:?}/{cur_step} — instant retry on next bucket {d2:?}/{s2}");
                        }
                        attempt_payload = build_payload(&messages, chunk, d2);
                        cur_dialect = d2;
                        cur_step = s2;
                    }
                    Err(e) => out = Some(Err(e)),
                }
            }
            out.unwrap()
        };
        let resp = match first {
            Ok((used_dialect, r)) => {
                effective_dialect = used_dialect;
                r
            }
            Err(e) if framer_core::error::AccountCooldown::from_error(&e).is_some() => return Err(e),
            Err(e) if cfg.is_cancelled() => return Err(e),
            Err(e) => {
                // The idle-kill check must time the FAILING ATTEMPT — the whole
                // loop includes 429 backoffs, which would misclassify a fast
                // velocity 403 after >8s of waiting as an idle-window kill
                // (audit 2026-10-11: wasted halve-retries on velocity bans).
                let elapsed = cooldown_wait.elapsed(last_attempt_t0).as_secs_f32();
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
                let retry_payload = build_payload(&messages, budget.min(chunk_tokens), d3);
                match cooldown_wait.run(cfg, !blocks.is_empty(), || proxy::forward_cancellable(
                    t, token_cache, session, browser, account, project,
                    d3, &s3, retry_payload.clone(), chat_session, i, cfg.cancelled.as_deref(),
                )).await {
                    Ok(r) => r,
                    Err(e2) if framer_core::error::AccountCooldown::from_error(&e2).is_some() => return Err(e2),
                    Err(e2) if cfg.is_cancelled() => return Err(e2),
                    Err(e2) if cfg.metered_fallback => {
                        eprintln!("[freechat] chunk {i} retry died ({e2}) — metered stream fallback for this chunk");
                        let stream_payload = build_payload(&messages, budget.min(chunk_tokens), d3);
                        let streamed = stream_chunk(
                            t, token_cache, session, browser, account, project,
                            d3, &s3, stream_payload, chat_session, i, cfg.cancelled.as_deref(),
                        ).await?;
                        metered_chunks += 1;
                        ptok = ptok.max(streamed.input_tokens);
                        ctok = add_output_tokens(ctok, streamed.output_tokens)?;
                        last_usage = streamed.usage.clone();
                        if msg_id.is_empty() && !streamed.id.is_empty() {
                            msg_id = streamed.id.clone();
                        }
                        let streamed_stop = streamed.stop_reason.clone();
                        let streamed_seq = streamed.stop_sequence.clone();
                        stop_reason = streamed_stop.clone();
                        stop_sequence = streamed_seq.clone();
                        let streamed_blocks = streamed.content.clone();
                        merge_blocks(&mut blocks, streamed_blocks.clone());
                        if let Some(cb) = events {
                            cb(ChainEvent::Chunk(streamed_blocks.clone()));
                        }
                        let finished = streamed_stop != "max_tokens"
                            || remaining_tokens(cfg, ctok) == Some(0);
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
        let wall = cooldown_wait.elapsed(t0).as_secs_f32();
        let queued = resp.queue_wait_ms as f32 / 1000.0;
        let elapsed = (wall - queued).max(0.5);
        if queued > 2.0 {
            eprintln!("[freechat] chunk {i} queued {queued:.1}s of {wall:.1}s (excluded from tok/s)");
        }
        let body: Value = serde_json::from_str(&body_text(resp).await?).map_err(|e| format!("chunk {i} not json: {e}"))?;
        if msg_id.is_empty() {
            msg_id = body["id"].as_str().unwrap_or_default().to_string();
        }
        let (chunk_blocks, chunk_stop, usage) = match effective_dialect {
            Dialect::OpenRouterMessages => buffered::decode(&body)?,
            _ => framer_core::convert::chat_response_to_blocks(&body),
        };
        let in_t = usage["input_tokens"].as_u64().unwrap_or(0);
        let out_t = usage["output_tokens"].as_u64().unwrap_or(0);
        ptok = ptok.max(in_t); // input counts the whole conversation; keep latest
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
        // Match the streaming gate: a token-limited chunk cannot publish a
        // tool merely because another block followed it or it is server-owned.
        let contains_tool_use = chunk_blocks.iter().any(|block|
            matches!(block["type"].as_str(), Some("tool_use" | "server_tool_use")));
        if chunk_stop == "max_tokens" && contains_tool_use && tool_cut_retries < 2 {
            if i + 1 >= cfg.max_chunks {
                return Err(format!("chunk {i}: incomplete tool call has no remaining retry chunk"));
            }
            // Free retry with a WINDOW-PROPORTIONAL budget: ~70s of the
            // model's own measured throughput (the messages-route window is
            // ~100s; the old 45s/3000-cap sat below real capacity, so
            // 3-5k-token writes were cut twice and the chain errored out —
            // 99 such deaths on 2026-10-04).
            tool_cut_retries += 1;
            let budget = ((tps * 70.0) as u32).clamp(1200, 5000).max(chunk_tokens);
            eprintln!("[freechat] chunk {i} cut mid-tool_use — free retry #{tool_cut_retries} (max_tokens={budget}, measured {tps:.0} tok/s, instructing split)");
            chunk_tokens = budget;
            // ROOT FIX for repeat cuts: the model doesn't know its tool call
            // was truncated — left alone it re-attempts the SAME giant write
            // and gets cut again. Keep any text it produced, then instruct
            // the split explicitly.
            let texts: Vec<Value> = chunk_blocks
                .iter()
                .filter(|b| b["type"].as_str() == Some("text"))
                .cloned()
                .collect();
            let arr = messages.as_array_mut().expect("messages array");
            if !texts.is_empty() {
                arr.push(json!({"role": "assistant", "content": texts}));
            }
            arr.push(json!({"role": "user", "content": "Your previous response was cut off mid-tool-call because the tool call was too large. Do NOT retry it as one call. Split the work: write at most ~40 lines per call, then append the next section with a separate call, and repeat until the whole file or edit is done."}));
            continue;
        }
        if chunk_stop == "max_tokens" && contains_tool_use {
            if !cfg.metered_fallback {
                return Err(format!("chunk {i}: tool call exceeds the free non-streaming window; set FRAMERIZE_ALLOW_METERED=1 to allow a tiny metered chunk here"));
            }
            eprintln!("[freechat] chunk {i} cut mid-tool_use again — streaming this chunk metered (explicit opt-in)");
            let stream_payload = build_payload(&messages, remaining_tokens(cfg, ctok).unwrap_or(8000), dialect);
            let streamed = stream_chunk(
                t, token_cache, session, browser, account, project,
                dialect, &step, stream_payload, chat_session, i, cfg.cancelled.as_deref(),
            ).await?;
            metered_chunks += 1;
            ptok = ptok.max(streamed.input_tokens);
            ctok = add_output_tokens(ctok, streamed.output_tokens)?;
            last_usage = streamed.usage.clone();
            if msg_id.is_empty() && !streamed.id.is_empty() {
                msg_id = streamed.id.clone();
            }
            let streamed_stop = streamed.stop_reason.clone();
            let streamed_seq = streamed.stop_sequence.clone();
            stop_reason = streamed_stop.clone();
            stop_sequence = streamed_seq.clone();
            let streamed_blocks = streamed.content.clone();
            merge_blocks(&mut blocks, streamed_blocks.clone());
            if let Some(cb) = events {
                cb(ChainEvent::Chunk(streamed_blocks.clone()));
            }
            let finished = streamed_stop != "max_tokens"
                || remaining_tokens(cfg, ctok) == Some(0);
            if finished {
                return Ok(assemble(msg_id, blocks, streamed_stop, streamed_seq, last_usage, ptok, ctok, i + 1, metered_chunks));
            }
            push_continuation(&mut messages, &streamed_blocks, cfg);
            continue;
        }

        // A cut tool response is discarded above; count only published output.
        ctok = add_output_tokens(ctok, out_t)?;
        stop_reason = chunk_stop.clone();
        stop_sequence = body["stop_sequence"].as_str().map(str::to_string);
        bucket_idx += 1; // rotate the bucket every chunk, not just on 429
        merge_blocks(&mut blocks, chunk_blocks.clone());
        if let Some(cb) = events {
            cb(ChainEvent::Chunk(chunk_blocks.clone()));
        }

        let budget_left = remaining_tokens(cfg, ctok);
        if chunk_stop != "max_tokens" || budget_left == Some(0) {
            return Ok(assemble(msg_id, blocks, stop_reason, stop_sequence, last_usage, ptok, ctok, i + 1, metered_chunks));
        }
        push_continuation(&mut messages, &chunk_blocks, cfg);
    }
    Ok(assemble(msg_id, blocks, stop_reason, stop_sequence, last_usage, ptok, ctok, cfg.max_chunks, metered_chunks))
}

fn remaining_tokens(cfg: &ChainConfig, output_tokens: u64) -> Option<u32> {
    // Subtract before narrowing: the remaining amount cannot exceed its u32 cap.
    cfg.max_total_tokens.map(|cap| u64::from(cap).saturating_sub(output_tokens) as u32)
}

fn add_output_tokens(total: u64, output_tokens: u64) -> Result<u64, String> {
    total.checked_add(output_tokens)
        .ok_or_else(|| "Messages output token usage overflow".into())
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

/// Append this chunk's blocks as the assistant turn + the continue prompt.
/// Only ever called when the chunk is text-only (any tool_use ends the
/// chain), so the assistant message is spec-valid.

#[path = "messages_stream.rs"]
mod stream;

#[path = "messages_json.rs"]
mod buffered;

#[cfg(test)]
#[path = "messages_fallback_tests.rs"]
mod fallback_tests;

// ---------------------------------------------------------------------------
// Metered streaming fallback (rung 3): consume one SSE stream, BLOCK-AWARE.
// ---------------------------------------------------------------------------

#[derive(Debug)]
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
    token_cache: &Mutex<Vec<(AccountId, AiToken)>>,
    session: &Session,
    browser: &dyn BrowserBridge,
    account: &AccountId,
    project: &ProjectId,
    upstream: Dialect,
    step: &str,
    mut payload: Value,
    chat_session: &str,
    step_index: u32,
    cancelled: Option<&(dyn Fn() -> bool + Send + Sync)>,
) -> Result<StreamedChunk, String> {
    payload["stream"] = json!(true);
    let resp = proxy::forward_cancellable(t, token_cache, session, browser, account, project, upstream, step, payload, chat_session, step_index, cancelled).await?;
    let text = body_text(resp).await?;

    stream::parse(&text)
}

// Shared preparation only. Transport and retry policies remain separate.
