//! The responses-native free chain (spec: docs/native-chains.md).
//!
//! Codex and other OpenAI-Responses clients run on `/ai/v3/responses/`
//! verbatim — the client payload rides upstream untouched (tools, reasoning,
//! service tiers, ids all survive), chunked only by budget. No Anthropic
//! conversion anywhere on this path (that conversion was the root cause of
//! the "no tools available" bug class).
//!
//! Continuation is client-side: `previous_response_id` is unsupported on the
//! proxy (measured 2026-10-06, 400), so each chunk's `output` array is
//! appended to `input` verbatim (reasoning items included — verified), plus a
//! "continue exactly where you stopped" user message.

use crate::protocol::responses::{merge_items, record_chunk_events, prepare_input, prepare_payload};
use std::sync::Mutex;

use serde_json::{json, Value};

use framer_core::convert::{AssembledResponses, ChainEvent};
use framer_core::ids::{AccountId, ProjectId};
use framer_core::port::{BrowserBridge, ChainConfig, Session};

use crate::auth::AiToken;
use crate::proxy::{self, Dialect};
use crate::transport::Transport;
use futures::StreamExt;

#[cfg(test)]
#[path = "../nativechain_tool_tests.rs"]
mod tool_completion_tests;

#[cfg(test)]
#[path = "responses_sse_tests.rs"]
mod responses_sse_tests;

#[cfg(test)]
#[path = "responses_fallback_tests.rs"]
mod responses_fallback_tests;

/// Fire a full (possibly multi-chunk) generation on the responses passthrough
/// and return the assembled native Responses object.
#[allow(clippy::too_many_arguments)]
pub async fn complete_long(
    t: &Transport,
    token_cache: &Mutex<Vec<(AccountId, AiToken)>>,
    session: &Session,
    browser: &dyn BrowserBridge,
    account: &AccountId,
    project: &ProjectId,
    client_payload: Value,
    cfg: &ChainConfig,
    chat_session: &str,
    events: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
) -> Result<AssembledResponses, String> {
    if cfg.max_total_tokens.is_some_and(|cap| cap < 16) {
        return Err("Responses output budget must be at least 16 tokens".into());
    }
    let chain_step: String =
        if std::env::var("FRAMERIZE_FEATURE_CATEGORY").unwrap_or_else(|_| "translation".to_string()) == "translation" {
            "translate".to_string()
        } else {
            "message".to_string()
        };
    let chain_nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() ^ ((d.subsec_nanos() as u64) << 20))
        .unwrap_or(0x9e3779b97f4a7c15);

    if cfg.max_chunks == 0 {
        return Err("Responses chain max_chunks must be greater than zero".into());
    }

    let mut input_items = prepare_input(&client_payload);

    // Per-chunk payload: client verbatim, model normalized, stream off,
    // budget set, proxy-unsupported persistence fields stripped (measured
    // 400s: previous_response_id, store, background).
    let build_payload = |input: &[Value], chunk_budget: u32| {
        prepare_payload(&client_payload, input, chunk_budget)
    };

    let mut output_items: Vec<Value> = Vec::new();
    let mut resp_id = String::new();
    let mut created_at = 0u64;
    let mut last_usage = json!({});
    let mut last_service_tier = Value::Null;
    let (mut ptok, mut ctok) = (0u64, 0u64);
    let mut metered_chunks = 0u32;
    let mut tool_cut_retries = 0u32;
    let mut slow_chunks = 0u32;
    let mut rate_retries = 0u32;
    let mut final_status = "completed".to_string();
    let mut final_incomplete: Option<Value> = None;

    let window_budget_s = 45.0f32;
    // Fast lane: the client explicitly opted into a speed tier (ultrafast/
    // priority/fast — the T3 -ultrafast variants inject these). At measured
    // 100-350 tok/s a 600-token first chunk means 4+ chunks per real
    // response, each paying TTFT + continuation + eval overhead (~2-3s) —
    // measured 2026-10-07: 752-token response, 105 tok/s chunk decode but
    // 77 tok/s wall. Fast-lane requests start near-single-shot and may grow
    // to fill the window at measured speed; if the tier sheds to default
    // the death-retry halves and re-measures, self-healing.
    let fast_lane = matches!(
        client_payload["service_tier"].as_str(),
        Some("ultrafast") | Some("priority") | Some("fast")
    );
    let grow_cap: u32 = if fast_lane { 12000 } else { 3000 };
    let chunk_cap = cfg.chunk_tokens.max(if fast_lane { 4000 } else { 2500 });
    let mut chunk_tokens = if fast_lane { cfg.chunk_tokens.clamp(1500, grow_cap) } else { cfg.chunk_tokens.min(600) };
    let budget_for = |tps: f32| -> u32 { ((tps * window_budget_s) as u32).clamp(400, grow_cap) };

    let mut cooldown_wait = super::cooldown_wait::CooldownWait::default();
    let chain_started = cooldown_wait.timer();
    let max_active = std::time::Duration::from_secs(300);

    for i in 0..cfg.max_chunks {
        cfg.check_cancelled()?;
        if cooldown_wait.elapsed(chain_started) > max_active {
            return Err(format!(
                "chain exceeded {}s active-time budget after {i} chunks (excluding local cooldown waits), retry the request",
                max_active.as_secs()
            ));
        }
        let budget = cfg
            .max_total_tokens
            .map(|cap| cap.saturating_sub(ctok as u32).max(1))
            .unwrap_or(chunk_tokens);
        let chunk = budget.min(chunk_tokens).min(chunk_cap).max(16);
        if i > 0 {
            // Human cadence between chunks (see freechat).
            let j = 700u64 + (chain_nonce.wrapping_add(i as u64).wrapping_mul(0x9e3779b1) % 1600);
            tokio::time::sleep(std::time::Duration::from_millis(j)).await;
        }
        let payload = build_payload(&input_items, chunk);
        let t0 = cooldown_wait.timer();
        eprintln!("[nativechain] chunk {i} start (max_output_tokens={chunk})");

        // 429 handling: instant in-chain retries (≤4) with the saturation
        // callback, then an honest error — same policy as freechat.
        let mut last_attempt_t0 = t0;
        let first = {
            let mut attempt_payload = payload;
            let mut out = None;
            while out.is_none() {
                cfg.check_cancelled()?;
                last_attempt_t0 = cooldown_wait.timer();
                match cooldown_wait.run(cfg, !output_items.is_empty(), || proxy::forward_cancellable(
                    t, token_cache, session, browser, account, project,
                    Dialect::OpenRouterResponses, &chain_step, attempt_payload.clone(),
                    chat_session, i, cfg.cancelled.as_deref(),
                ))
                .await
                {
                    Ok(r) => out = Some(Ok(r)),
                    Err(e) if cfg.is_cancelled() => return Err(e),
                    Err(e) if e.starts_with("retryable 429") => {
                        if rate_retries >= 9 {
                            out = Some(Err(format!(
                                "chunk {i}: workspace still saturated after {rate_retries} retries ({e})"
                            )));
                            break;
                        }
                        rate_retries += 1;
                        if let Some(cb) = &cfg.on_429 {
                            cb();
                        }
                        if rate_retries > 3 {
                            let wait = std::time::Duration::from_millis(
                                1500 * (1 << (rate_retries - 3).min(4)),
                            );
                            eprintln!("[nativechain] chunk {i} 429 — backoff {wait:?} (attempt {rate_retries})");
                            tokio::time::sleep(wait).await;
                        } else {
                            eprintln!("[nativechain] chunk {i} 429 — instant retry ({rate_retries})");
                        }
                        attempt_payload = build_payload(&input_items, chunk);
                    }
                    Err(e) => out = Some(Err(e)),
                }
            }
            out.unwrap()
        };

        // Keep the successful request's actual budget with its response.
        // A transport retry can lower it after the original payload was sent.
        let (resp, response_budget, streamed) = match first {
            Ok(r) => (r, chunk, false),
            Err(e) if framer_core::error::AccountCooldown::from_error(&e).is_some() => return Err(e),
            Err(e) if cfg.is_cancelled() => return Err(e),
            Err(e) => {
                // The idle-kill check must time the FAILING ATTEMPT — the whole
                // loop includes 429 backoffs, which would misclassify a fast
                // velocity 403 after >8s of waiting as an idle-window kill
                // (audit 2026-10-11: wasted halve-retries on velocity bans).
                let elapsed = cooldown_wait.elapsed(last_attempt_t0).as_secs_f32();
                let idle_killed = e == "risk-refused" && elapsed > 8.0;
                let upstream_refused = e.starts_with("upstream 403");
                let transport_died = !upstream_refused
                    && e != "risk-refused"
                    && e != "credits-exhausted"
                    && e != "unauthorized"
                    && !e.starts_with("retryable 4");
                if !((idle_killed || transport_died) && chunk_tokens > 200) {
                    return Err(e);
                }
                // Died silently (idle window or transport): halve and retry once.
                chunk_tokens /= 2;
                let retry_chunk = budget.min(chunk_tokens).min(chunk_cap).max(16);
                eprintln!("[nativechain] chunk {i} died at {elapsed:.1}s ({e}) — retrying with max_output_tokens={retry_chunk}");
                let retry_payload = build_payload(&input_items, retry_chunk);
                match cooldown_wait.run(cfg, !output_items.is_empty(), || proxy::forward_cancellable(
                    t, token_cache, session, browser, account, project,
                    Dialect::OpenRouterResponses, &chain_step, retry_payload.clone(), chat_session, i, cfg.cancelled.as_deref(),
                ))
                .await
                {
                    Ok(r) => (r, retry_chunk, false),
                    Err(e2) if framer_core::error::AccountCooldown::from_error(&e2).is_some() => return Err(e2),
                    Err(e2) if cfg.is_cancelled() => return Err(e2),
                    Err(e2) if cfg.metered_fallback => {
                        eprintln!("[nativechain] chunk {i} retry died ({e2}) — metered stream fallback for this chunk");
                        let sp = build_payload(&input_items, retry_chunk);
                        let streamed =
                            stream_chunk(t, token_cache, session, browser, account, project, &chain_step, sp, chat_session, i, cfg.cancelled.as_deref()).await?;
                        metered_chunks += 1;
                        // Apply the same tool, budget and continuation guards
                        // before publishing output from either transport mode.
                        (streamed, retry_chunk, true)
                    }
                    Err(e2) => return Err(format!("chunk {i} retry died (fallback off): {e2}")),
                }
            }
        };

        let wall = cooldown_wait.elapsed(t0).as_secs_f32();
        let queued = resp.queue_wait_ms as f32 / 1000.0;
        let elapsed = (wall - queued).max(0.5);
        if queued > 2.0 {
            eprintln!("[nativechain] chunk {i} queued {queued:.1}s of {wall:.1}s (excluded from tok/s)");
        }
        let text = body_text(resp).await?;
        let body: Value = if streamed {
            let chunk = parse_streamed_chunk(&text)?;
            let mut body = json!({
                "id": chunk.id, "created_at": chunk.created_at,
                "output": chunk.output, "status": chunk.status,
                "usage": chunk.usage, "service_tier": chunk.service_tier,
            });
            if let Some(details) = chunk.incomplete_details {
                body["incomplete_details"] = details;
            }
            body
        } else {
            let body: Value = serde_json::from_str(&text)
                .map_err(|e| format!("chunk {i} not json: {e}"))?;
            validate_response_body(&body)
                .map_err(|problem| format!("response chunk has {problem}"))?;
            body
        };

        let status = body["status"].as_str().unwrap_or("completed").to_string();
        let incomplete_reason = body["incomplete_details"]["reason"].as_str().map(str::to_string);
        let items = body["output"].as_array().cloned().unwrap_or_default();
        let usage = body.get("usage").cloned().unwrap_or_else(|| json!({}));
        let out_t = usage["output_tokens"].as_u64().unwrap_or(0);

        let tps = out_t as f32 / elapsed.max(0.5);
        eprintln!("[nativechain] chunk {i} done in {elapsed:.1}s ({out_t} tok, {tps:.0} tok/s, status {status})");
        // Failed attempts before streaming do not measure stream throughput.
        // Keep that recovery path outside the non-stream speed heuristic.
        if !streamed {
            if tps > 1.0 {
                chunk_tokens = budget_for(tps);
            }
            if tps > 0.0 && tps < 8.0 {
                slow_chunks += 1;
                if slow_chunks >= 2 {
                    return Err(format!("upstream degraded ({tps:.0} tok/s sustained) — retry in a minute"));
                }
            } else if tps >= 12.0 {
                slow_chunks = 0;
            }
        }

        let ends_in_call = items
            .last()
            .map(|it| matches!(it["type"].as_str(), Some("function_call") | Some("custom_tool_call")))
            .unwrap_or(false);
        // Live cut-off tool inputs were labelled completed at exactly the
        // requested 1500-token budget. That status can also be our default
        // when the upstream field is absent. Do not publish executable tool
        // input whose completeness is uncertain at the token boundary.
        let tool_budget_exhausted = ends_in_call && out_t >= u64::from(response_budget);
        let hit_cap = (status == "incomplete"
            && incomplete_reason.as_deref() == Some("max_output_tokens"))
            || tool_budget_exhausted;
        if tool_budget_exhausted && status != "incomplete" {
            eprintln!("[nativechain] chunk {i} tool output exhausted {response_budget}-token budget (status {status}); withholding for retry");
        }

        // Mid-tool-call cut: same policy as freechat — free retry with a
        // window-proportional budget and an explicit split instruction (the
        // truncated call is dropped; its text survives as context).
        if hit_cap && ends_in_call && tool_cut_retries < 2 {
            if i + 1 >= cfg.max_chunks {
                return Err(format!("chunk {i}: incomplete tool call has no remaining retry chunk"));
            }
            tool_cut_retries += 1;
            let budget = ((tps * 70.0) as u32).clamp(1200, 5000).max(chunk_tokens);
            eprintln!("[nativechain] chunk {i} cut mid-tool-call — free retry #{tool_cut_retries} (max_output_tokens={budget}, {tps:.0} tok/s, instructing split)");
            chunk_tokens = budget;
            // This discarded chunk has not been added to ctok. Previously
            // subtracting it erased the usage of already delivered chunks.
            let text_items: Vec<Value> = items
                .iter()
                .filter(|it| it["type"].as_str() == Some("message") || it["type"].as_str() == Some("reasoning"))
                .cloned()
                .collect();
            input_items.extend(text_items);
            input_items.push(json!({
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "Your previous response was cut off mid-tool-call because the tool call was too large. Do NOT retry it as one call. Split the work: write at most ~40 lines per call, then append the next section with a separate call, and repeat until the whole file or edit is done."}],
            }));
            continue;
        }
        if hit_cap && ends_in_call && tool_cut_retries >= 2 {
            return Err(format!(
                "chunk {i}: tool call is still incomplete after two bounded retries; split the tool input into smaller calls"
            ));
        }

        // Refuse unrepresentable usage before publishing this chunk's items.
        let next_ptok = ptok.max(usage["input_tokens"].as_u64().unwrap_or(0));
        let next_ctok = ctok.checked_add(out_t).ok_or("Responses output usage overflow")?;
        next_ptok.checked_add(next_ctok).ok_or("Responses total usage overflow")?;

        // Normal chunk completion: merge items, emit, decide continuation.
        let continues_text = hit_cap
            && items.last().map(|it| it["type"].as_str() == Some("message")).unwrap_or(false);
        record_chunk_events(&items, continues_text, events);
        merge_items(&mut output_items, items.clone());
        if resp_id.is_empty() {
            resp_id = body["id"].as_str().unwrap_or_default().to_string();
            created_at = body["created_at"].as_u64().unwrap_or(0);
        }
        ptok = next_ptok;
        ctok = next_ctok;
        last_usage = usage.clone();
        if !body["service_tier"].is_null() {
            last_service_tier = body["service_tier"].clone();
        }
        final_status = status.clone();
        final_incomplete = body.get("incomplete_details").cloned();

        let budget_left = cfg.max_total_tokens.map(|cap| u64::from(cap).saturating_sub(ctok));
        // prepare_payload enforces the upstream 16-token minimum. Do not
        // round a smaller remainder up beyond the client's total cap.
        if !hit_cap || budget_left.is_some_and(|left| left < 16) {
            return finish(
                resp_id, created_at, output_items, final_status, final_incomplete,
                last_usage, last_service_tier, ptok, ctok, i + 1, metered_chunks,
            );
        }
        // Continuation: append the chunk's whole output verbatim (reasoning
        // included — verified) + the continue instruction.
        input_items.extend(items);
        input_items.push(json!({
            "type": "message", "role": "user",
            "content": [{"type": "input_text", "text": "Continue exactly where you stopped. Do not repeat anything."}],
        }));
    }
    finish(
        resp_id, created_at, output_items, final_status, final_incomplete,
        last_usage, last_service_tier, ptok, ctok, cfg.max_chunks, metered_chunks,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish(
    id: String,
    created_at: u64,
    output: Vec<Value>,
    status: String,
    incomplete_details: Option<Value>,
    usage: Value,
    service_tier: Value,
    input_tokens: u64,
    output_tokens: u64,
    chunks: u32,
    metered_chunks: u32,
) -> Result<AssembledResponses, String> {
    Ok(AssembledResponses {
        id,
        created_at,
        output,
        status,
        incomplete_details,
        usage,
        service_tier,
        input_tokens,
        output_tokens,
        chunks,
        metered_chunks,
    })
}

async fn body_text(resp: framer_core::port::ProxyResponse) -> Result<String, String> {
    let mut stream = resp.stream;
    let mut buf = bytes::BytesMut::new();
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk.map_err(|e| e.to_string())?);
    }
    String::from_utf8(buf.to_vec()).map_err(|e| e.to_string())
}

/// Request an SSE chunk without bypassing the caller's completion checks.
#[allow(clippy::too_many_arguments)]
async fn stream_chunk(
    t: &Transport,
    token_cache: &Mutex<Vec<(AccountId, AiToken)>>,
    session: &Session,
    browser: &dyn BrowserBridge,
    account: &AccountId,
    project: &ProjectId,
    step: &str,
    mut payload: Value,
    chat_session: &str,
    step_index: u32,
    cancelled: Option<&(dyn Fn() -> bool + Send + Sync)>,
) -> Result<framer_core::port::ProxyResponse, String> {
    payload["stream"] = json!(true);
    proxy::forward_cancellable(
        t, token_cache, session, browser, account, project,
        Dialect::OpenRouterResponses, step, payload, chat_session, step_index, cancelled,
    )
    .await
}

fn validate_response_body(body: &Value) -> Result<(), &'static str> {
    if !body.is_object() {
        return Err("invalid response object");
    }
    if !body["output"].is_array() {
        return Err("invalid output array");
    }
    if body.get("status").is_some_and(|status| {
        !matches!(status.as_str(), Some("completed" | "incomplete"))
    }) {
        return Err("invalid terminal status");
    }
    if body.get("usage").is_some_and(|usage| !usage.is_null() && !usage.is_object()) {
        return Err("invalid usage object");
    }
    Ok(())
}

fn parse_streamed_chunk(text: &str) -> Result<AssembledResponses, String> {
    // The buffered SSE text: find the terminal response.completed event.
    let mut final_obj: Option<Value> = None;
    let mut data_lines = Vec::new();
    for line in text.lines().chain(std::iter::once("")) {
        if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.strip_prefix(' ').unwrap_or(data));
            continue;
        }
        if !line.is_empty() || data_lines.is_empty() {
            continue;
        }
        let data = data_lines.join("\n");
        data_lines.clear();
        if data == "[DONE]" {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(&data) {
            match v["type"].as_str() {
                Some("response.completed") => final_obj = v.get("response").cloned(),
                Some(kind @ ("error" | "response.failed" | "response.incomplete")) => {
                    return Err(format!("streamed chunk ended with {kind}"));
                }
                _ => {}
            }
        }
    }
    let body = final_obj.ok_or("streamed chunk had no response.completed event")?;
    validate_response_body(&body).map_err(|problem| format!("streamed chunk has {problem}"))?;
    let output = body["output"].as_array().expect("validated response output").clone();
    let usage = body.get("usage").cloned().unwrap_or_else(|| json!({}));
    Ok(AssembledResponses {
        id: body["id"].as_str().unwrap_or_default().to_string(),
        created_at: body["created_at"].as_u64().unwrap_or(0),
        output,
        status: body["status"].as_str().unwrap_or("completed").to_string(),
        incomplete_details: body.get("incomplete_details").cloned(),
        service_tier: body["service_tier"].clone(),
        input_tokens: usage["input_tokens"].as_u64().unwrap_or(0),
        output_tokens: usage["output_tokens"].as_u64().unwrap_or(0),
        usage,
        chunks: 1,
        metered_chunks: 1,
    })
}

// Preserve the existing typed-input and namespace normalization.

// Shared preparation only. Transport and retry policies remain separate.
