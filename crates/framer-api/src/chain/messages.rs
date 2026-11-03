//! Bounded continuations on the Anthropic-format passthrough.
//! All models share block validation, history, usage and termination logic.
//! Native request fields and signed blocks are retained. Only complete tool
//! calls are published; acknowledged truncations can retry unpublished input.
//!
//! Verified on the live account (verified-captures-2026-10-01.md):
//!   - non-streaming on /ai/v3/messages/ is unmetered; streaming is metered.
//!   - the messages route's idle window is ~60-109s (chat route: ~21.5s).
//! Unknown transport completion is terminal. Metered streaming requires the
//! caller's explicit permission and never repairs an unknown completion.

use crate::protocol::messages::prepare_payload;
use std::sync::Mutex;

use serde_json::{json, Value};

use framer_core::convert::{AssembledResponse, ChainEvent};
use framer_core::ids::{AccountId, ProjectId};
use framer_core::port::{BrowserBridge, ChainConfig, Session};

use crate::proxy::{self, Dialect};
use crate::auth::AiToken;
use crate::transport::Transport;
use futures::StreamExt;

#[path = "messages_checkpoint.rs"]
pub(super) mod checkpoint;

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
    mut anthro_payload: Value,
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
    t.messages_history.restore(&mut anthro_payload)?;
    let mut history = super::messages_history::Trace::default();
    let output_floor = framer_core::convert::anthropic_output_floor(&anthro_payload)?;
    let explicit_reasoning = matches!(anthro_payload["thinking"]["type"].as_str(),
        Some("adaptive" | "enabled"));
    let thinking_mode = match anthro_payload["thinking"]["type"].as_str() {
        Some("adaptive") => "adaptive thinking",
        Some("enabled") => "enabled thinking",
        Some("disabled") => "disabled thinking",
        _ => "default reasoning",
    };
    if cfg.max_total_tokens.is_some_and(|total| total < output_floor) {
        return Err("Messages total output limit cannot accommodate the enabled thinking budget".into());
    }
    // Keep the provider route and step fixed throughout the request.
    let dialect = Dialect::OpenRouterMessages;
    let chain_nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() ^ ((d.subsec_nanos() as u64) << 20))
        .unwrap_or(0x9e3779b97f4a7c15);
    // Match the existing envelope category and its default.
    let category = std::env::var("FRAMERIZE_FEATURE_CATEGORY")
        .unwrap_or_else(|_| "translation".to_string());
    let step: String =
        if category == "translation" { "translate".to_string() } else { "message".to_string() };

    let mut streaming = explicit_reasoning && cfg.metered_fallback;

    let mut messages = anthro_payload["messages"].clone();
    let mut blocks: Vec<Value> = Vec::new();
    let mut msg_id = String::new();
    let mut stop_reason = "end_turn".to_string();
    let mut stop_sequence: Option<String> = None;
    let mut stop_details: Option<Value> = None;
    let mut last_usage = json!({});
    let (mut ptok, mut ctok) = (0u64, 0u64);
    let mut metered_chunks = 0u32;
    let mut tool_cut_retries = 0u32;
    let mut rate_retries = 0u32;

    // Start small, then use observed throughput to target 45-second chunks.
    // Token limits cannot guarantee a deadline, especially when an explicit
    // thinking budget requires a larger per-request output allowance.
    let window_budget_s = 45.0f32;
    let mut chunk_tokens = cfg.chunk_tokens.min(600);
    let chunk_ceiling = 1200;
    let budget_for = |tps: f32| -> u32 { ((tps * window_budget_s) as u32).clamp(400, chunk_ceiling) };

    let mut cooldown_wait = cfg.admission.as_ref().map_or_else(
        super::cooldown_wait::CooldownWait::default,
        |guard| super::cooldown_wait::CooldownWait::with_limit(guard.wait_budget()));
    let active_budget = framer_protocol::active_budget::ActiveBudget::new(&cooldown_wait);
    for i in 0..cfg.max_chunks {
        cfg.check_cancelled()?;
        active_budget.check(&cooldown_wait, i, None)?;
        let budget = remaining_tokens(cfg, ctok).unwrap_or(u32::MAX);
        // Every Messages model uses the same bounded, block-aware continuation.
        let chunk = if streaming {
            budget.min(cfg.max_total_tokens.unwrap_or(cfg.chunk_tokens).max(output_floor))
        } else { budget.min(chunk_tokens.max(output_floor)) };
        // Retain the existing 0.7-2.3 second pacing between requests.
        if i > 0 {
            let j = 700u64 + (chain_nonce.wrapping_add(i as u64).wrapping_mul(0x9e3779b1) % 1600);
            active_budget.pause(&cooldown_wait, cfg,
                std::time::Duration::from_millis(j), i, None).await?;
        }
        let mut payload = prepare_payload(&anthro_payload, &messages, chunk, dialect);
        payload["stream"] = json!(streaming);
        let t0 = cooldown_wait.timer();
        eprintln!("[freechat] chunk {i} start (max_tokens={chunk}, {dialect:?}/{step})");
        // Rate-limit retries retain this request's route, payload and identity.
        // A provider limit does not establish workspace saturation.
        let mut last_attempt_t0 = t0;
        let first = {
            let cur_dialect = dialect;
            let cur_step = step.clone();
            let attempt_payload = payload;
            let mut out = None;
            while out.is_none() {
                cfg.check_cancelled()?;
                active_budget.check(&cooldown_wait, i, None)?;
                last_attempt_t0 = cooldown_wait.timer();
                let attempt = || async {
                    if let Some(guard) = &cfg.admission { guard.before_attempt().await?; }
                    let result = proxy::forward_cancellable(
                        t, token_cache, session, browser, account, project,
                        cur_dialect, &cur_step, attempt_payload.clone(), chat_session, i,
                        cfg.cancelled.as_deref(),
                    ).await;
                    if let (Some(guard), Err(error)) = (&cfg.admission, &result) {
                        if framer_core::error::is_security_refusal(error)
                            || framer_core::error::AccountCooldown::from_error(error).is_some() {
                            guard.refused(error).await?;
                        }
                    }
                    result
                };
                let result = if cfg.admission.is_some() {
                    cooldown_wait.run_guarded(cfg, &active_budget, i, attempt).await
                } else {
                    cooldown_wait.run_budgeted(cfg, !blocks.is_empty(), &active_budget, i, attempt).await
                };
                match result {
                    Ok(r) => out = Some(Ok(r)),
                    Err(e) if cfg.is_cancelled() => return Err(e),
                    Err(e) if !streaming && e.starts_with("retryable 429") => {
                        if rate_retries >= 9 {
                            out = Some(Err(e));
                            break;
                        }
                        rate_retries += 1;
                        if framer_core::error::is_workspace_saturation(&e) {
                            if let Some(cb) = &cfg.on_429 { cb(); }
                        }
                        let wait = std::time::Duration::from_millis(
                            1500 * (1 << (rate_retries - 1).min(4)));
                        eprintln!("[freechat] chunk {i} HTTP 429, waiting {wait:?} (attempt {rate_retries}/9)");
                        active_budget.pause(&cooldown_wait, cfg, wait, i, Some(&e)).await?;
                    }
                    Err(e) => out = Some(Err(e)),
                }
            }
            out.unwrap()
        };
        let resp = match first {
            Ok(r) => r,
            Err(e) if framer_core::error::AccountCooldown::from_error(&e).is_some() => return Err(e),
            Err(e) if cfg.is_cancelled() => return Err(e),
            Err(e) if !streaming && (e.starts_with("in-page fetch failed:")
                || matches!(framer_core::convert::parse_upstream_error(&e), Some((502 | 504, _)))) => {
                let elapsed = cooldown_wait.elapsed(last_attempt_t0).as_secs_f32();
                let cause = framer_core::error::safe_failure_summary(&e);
                let prefix = framer_core::error::ADAPTIVE_INCOMPLETE_PREFIX;
                return Err(format!("{prefix}Claude's buffered upstream request failed after {elapsed:.1}s before a complete response was available. The request allowed {chunk} output tokens with {thinking_mode}. Recorded cause: {cause}. Automatic replay is disabled because completion is unknown. This custom-route request was not switched to metered streaming."));
            }
            Err(e) if framer_core::convert::parse_upstream_error(&e).is_none()
                && !framer_core::error::is_local_request_failure(&e)
                && !framer_core::error::is_security_refusal(&e)
                && !e.starts_with("server-cooling:") && !e.starts_with("chain exceeded ")
                && !matches!(e.as_str(), "unauthorized" | "credits-exhausted") =>
                    return Err(unacknowledged(&e)),
            Err(e) => return Err(e),
        };

        // Wall-clock includes OUR queue wait for a free browser page. Subtract
        // it: queue wait and a slow upstream look identical in wall-clock, and
        // charging queue time to the upstream makes the sizer below shrink
        // chunks under concurrency -- which multiplies round trips (every chunk
        // re-sends the whole conversation) and deepens the very queue it is
        // reacting to. Measured 2026-10-03: 85 tok/s solo -> 3-6 tok/s at 12
        // agents on an otherwise IDLE box (load average 0.29).
        let wall = cooldown_wait.elapsed(last_attempt_t0).as_secs_f32();
        let queued = resp.queue_wait_ms as f32 / 1000.0;
        let elapsed = (wall - queued).max(0.5);
        if queued > 2.0 {
            eprintln!("[freechat] chunk {i} queued {queued:.1}s of {wall:.1}s (excluded from tok/s)");
        }
        let completed = body_text(resp).await.and_then(|text| decode_chunk(&text, streaming))
            .map_err(|error| if framer_core::convert::parse_upstream_error(&error).is_some() {
                error
            } else { unacknowledged(&error) })?;
        if streaming { metered_chunks += 1; }
        if msg_id.is_empty() {
            msg_id = completed.id.clone();
        }
        let (chunk_blocks, chunk_stop) = (completed.content, completed.stop_reason);
        let (in_t, out_t) = (completed.input_tokens, completed.output_tokens);
        ptok = ptok.max(in_t); // input counts the whole conversation; keep latest
        last_usage = completed.usage;
        let contains_tool_use = chunk_blocks.iter().any(|block|
            matches!(block["type"].as_str(), Some("tool_use" | "server_tool_use")));
        // Estimate the next allowance within the 400-1,200 token range.
        let tps = out_t as f32 / elapsed.max(0.5);
        eprintln!("[freechat] chunk {i} done in {elapsed:.1}s ({out_t} tok, {tps:.0} tok/s)");
        if tps > 1.0 {
            chunk_tokens = budget_for(tps);
        }
        // Keep valid output at low throughput. The active-time budget limits
        // later work without discarding an already successful response.

        if chunk_stop == "max_tokens" && contains_tool_use {
            if i + 1 >= cfg.max_chunks {
                return Err("Claude incomplete tool call has no remaining retry chunk".into());
            }
            if streaming || (tool_cut_retries >= 2 && !cfg.metered_fallback)
                || chunk_blocks.iter().any(|b| b["type"] == "server_tool_use") {
                return Err("Claude reached the checkpoint output limit during a tool call; complete tool input is unavailable".into());
            }
            // Later thinking can be bound to a preceding tool block. Retain
            // the exact prefix before the first unpublished tool, not gaps.
            let completed: Vec<Value> = chunk_blocks.iter()
                .take_while(|b| b["type"] != "tool_use").cloned().collect();
            if !completed.is_empty() && !checkpoint::available(&completed)? {
                return Err("Claude reached the checkpoint output limit during a tool call without a signed or text checkpoint".into());
            }
            ctok = add_output_tokens(ctok, out_t)?;
            if remaining_tokens(cfg, ctok).is_some_and(|left| left < output_floor) {
                return Err("Claude exhausted the requested output allowance before completing its tool input".into());
            }
            tool_cut_retries += 1;
            let next = remaining_tokens(cfg, ctok).unwrap_or(u32::MAX)
                .min(chunk.saturating_mul(2)).min(chunk_ceiling.max(output_floor));
            if completed.is_empty() || tool_cut_retries > 2 {
                if tool_cut_retries > 2 || next <= chunk {
                    if !cfg.metered_fallback {
                        return Err("Claude tool input exceeds the bounded output allowance; incomplete input was not published".into());
                    }
                    streaming = true;
                }
                if completed.is_empty() {
                    // The provider acknowledged max_tokens. No tool executed.
                    // Retry only that unpublished output, with identical context.
                    chunk_tokens = next;
                    continue;
                }
            }
            eprintln!("[freechat] chunk {i} completed at max_tokens with incomplete tool input; continuing from preserved checkpoint");
            history.append(&t.messages_history, &mut blocks, &completed)?;
            if let Some(cb) = events { cb(ChainEvent::Chunk(completed.clone())); }
            checkpoint::push(&mut messages, &completed, checkpoint::TOOL_CONTINUE);
            history.continuing(true);
            chunk_tokens = chunk_tokens.max(budget_for(tps));
            continue;
        }

        let has_checkpoint = chunk_stop == "max_tokens"
            && checkpoint::available(&chunk_blocks)?;
        // Count this completed response before deciding whether to continue.
        ctok = add_output_tokens(ctok, out_t)?;
        stop_reason = chunk_stop.clone();
        stop_sequence = completed.stop_sequence;
        stop_details = completed.stop_details;
        history.append(&t.messages_history, &mut blocks, &chunk_blocks)?;
        if let Some(cb) = events {
            cb(ChainEvent::Chunk(chunk_blocks.clone()));
        }

        let budget_left = remaining_tokens(cfg, ctok);
        if chunk_stop != "max_tokens" || budget_left.is_some_and(|left| left < output_floor)
            || !has_checkpoint {
            return Ok(assemble(msg_id, blocks, stop_reason, stop_sequence, stop_details, last_usage, ptok, ctok, i + 1, metered_chunks));
        }
        checkpoint::push(&mut messages, &chunk_blocks, checkpoint::CONTINUE);
        history.continuing(false);
    }
    Ok(assemble(msg_id, blocks, stop_reason, stop_sequence, stop_details, last_usage, ptok, ctok, cfg.max_chunks, metered_chunks))
}

fn remaining_tokens(cfg: &ChainConfig, output_tokens: u64) -> Option<u32> {
    // Subtract before narrowing: the remaining amount cannot exceed its u32 cap.
    cfg.max_total_tokens.map(|cap| u64::from(cap).saturating_sub(output_tokens) as u32)
}

fn unacknowledged(error: &str) -> String {
    format!("{}Claude did not return a complete, acknowledged response. Recorded cause: {}. Automatic replay is disabled because completion is unknown.",
        framer_core::error::ADAPTIVE_INCOMPLETE_PREFIX,
        framer_core::error::safe_failure_summary(error))
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
    stop_details: Option<Value>,
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
        stop_details,
        usage,
        input_tokens: ptok,
        output_tokens: ctok,
        chunks,
        metered_chunks,
    }
}

#[path = "messages_stream.rs"]
mod stream;

#[path = "messages_json.rs"]
mod buffered;

#[cfg(test)]
#[path = "messages_fallback_tests.rs"]
mod fallback_tests;

#[derive(Debug)]
struct StreamedChunk {
    id: String,
    content: Vec<Value>,
    stop_reason: String,
    stop_sequence: Option<String>,
    stop_details: Option<Value>,
    usage: Value,
    input_tokens: u64,
    output_tokens: u64,
}

fn decode_chunk(text: &str, streaming: bool) -> Result<StreamedChunk, String> {
    if streaming { return stream::parse(text); }
    let body: Value = serde_json::from_str(text).map_err(|e| format!("Messages response is not JSON: {e}"))?;
    let (content, stop_reason, usage) = buffered::decode(&body)?;
    Ok(StreamedChunk {
        id: body["id"].as_str().unwrap_or_default().into(), content, stop_reason,
        stop_sequence: body["stop_sequence"].as_str().map(str::to_string),
        stop_details: buffered::stop_details(&body)?,
        input_tokens: usage["input_tokens"].as_u64().unwrap_or(0),
        output_tokens: usage["output_tokens"].as_u64().unwrap_or(0), usage,
    })
}

// Shared preparation only. Transport and retry policies remain separate.
