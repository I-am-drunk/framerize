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
#[path = "nativechain_tool_tests.rs"]
mod tool_completion_tests;

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

    let chain_started = std::time::Instant::now();
    let max_wall = std::time::Duration::from_secs(300);

    for i in 0..cfg.max_chunks {
        cfg.check_cancelled()?;
        if chain_started.elapsed() > max_wall {
            return Err(format!(
                "chain exceeded {}s wall-clock budget after {i} chunks (upstream degraded) — retry the request",
                max_wall.as_secs()
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
        let t0 = std::time::Instant::now();
        eprintln!("[nativechain] chunk {i} start (max_output_tokens={chunk})");

        // 429 handling: instant in-chain retries (≤4) with the saturation
        // callback, then an honest error — same policy as freechat.
        let mut last_attempt_t0 = t0;
        let first = {
            let mut attempt_payload = payload;
            let mut out = None;
            while out.is_none() {
                cfg.check_cancelled()?;
                last_attempt_t0 = std::time::Instant::now();
                match proxy::forward_cancellable(
                    t, token_cache, session, browser, account, project,
                    Dialect::OpenRouterResponses, &chain_step, attempt_payload.take(),
                    chat_session, i, cfg.cancelled.as_deref(),
                )
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
        let (resp, response_budget) = match first {
            Ok(r) => (r, chunk),
            Err(e) if framer_core::error::AccountCooldown::from_error(&e).is_some() => return Err(e),
            Err(e) if cfg.is_cancelled() => return Err(e),
            Err(e) => {
                // The idle-kill check must time the FAILING ATTEMPT — the whole
                // loop includes 429 backoffs, which would misclassify a fast
                // velocity 403 after >8s of waiting as an idle-window kill
                // (audit 2026-10-11: wasted halve-retries on velocity bans).
                let elapsed = last_attempt_t0.elapsed().as_secs_f32();
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
                match proxy::forward_cancellable(
                    t, token_cache, session, browser, account, project,
                    Dialect::OpenRouterResponses, &chain_step, retry_payload, chat_session, i, cfg.cancelled.as_deref(),
                )
                .await
                {
                    Ok(r) => (r, retry_chunk),
                    Err(e2) if framer_core::error::AccountCooldown::from_error(&e2).is_some() => return Err(e2),
                    Err(e2) if cfg.is_cancelled() => return Err(e2),
                    Err(e2) if cfg.metered_fallback => {
                        eprintln!("[nativechain] chunk {i} retry died ({e2}) — metered stream fallback for this chunk");
                        let sp = build_payload(&input_items, retry_chunk);
                        let streamed =
                            stream_chunk(t, token_cache, session, browser, account, project, &chain_step, sp, chat_session, i, cfg.cancelled.as_deref()).await?;
                        metered_chunks += 1;
                        record_chunk(
                            streamed, &mut output_items, &mut resp_id, &mut created_at,
                            &mut last_usage, &mut last_service_tier, &mut ptok, &mut ctok,
                            &mut input_items, events,
                        );
                        return finish(
                            resp_id, created_at, output_items, "completed".into(), None,
                            last_usage, last_service_tier, ptok, ctok, i + 1, metered_chunks,
                        );
                    }
                    Err(e2) => return Err(format!("chunk {i} retry died (fallback off): {e2}")),
                }
            }
        };

        let wall = t0.elapsed().as_secs_f32();
        let queued = resp.queue_wait_ms as f32 / 1000.0;
        let elapsed = (wall - queued).max(0.5);
        if queued > 2.0 {
            eprintln!("[nativechain] chunk {i} queued {queued:.1}s of {wall:.1}s (excluded from tok/s)");
        }
        let body: Value = serde_json::from_str(&body_text(resp).await?)
            .map_err(|e| format!("chunk {i} not json: {e}"))?;

        let status = body["status"].as_str().unwrap_or("completed").to_string();
        let incomplete_reason = body["incomplete_details"]["reason"].as_str().map(str::to_string);
        let items = body["output"].as_array().cloned().unwrap_or_default();
        let usage = body.get("usage").cloned().unwrap_or_else(|| json!({}));
        let out_t = usage["output_tokens"].as_u64().unwrap_or(0);

        let tps = out_t as f32 / elapsed.max(0.5);
        eprintln!("[nativechain] chunk {i} done in {elapsed:.1}s ({out_t} tok, {tps:.0} tok/s, status {status})");
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

        // Normal chunk completion: merge items, emit, decide continuation.
        let continues_text = hit_cap
            && items.last().map(|it| it["type"].as_str() == Some("message")).unwrap_or(false);
        record_chunk_events(&items, continues_text, events);
        merge_items(&mut output_items, items.clone());
        if resp_id.is_empty() {
            resp_id = body["id"].as_str().unwrap_or_default().to_string();
            created_at = body["created_at"].as_u64().unwrap_or(0);
        }
        ptok = ptok.max(usage["input_tokens"].as_u64().unwrap_or(0));
        ctok += out_t;
        last_usage = usage.clone();
        if !body["service_tier"].is_null() {
            last_service_tier = body["service_tier"].clone();
        }
        final_status = status.clone();
        final_incomplete = body.get("incomplete_details").cloned();

        let budget_left = cfg.max_total_tokens.map(|cap| cap.saturating_sub(ctok as u32));
        if !hit_cap || budget_left == Some(0) {
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

/// Merge one chunk's output items into the total: consecutive assistant
/// message items merge (text continues); everything else appends.
fn merge_items(acc: &mut Vec<Value>, new: Vec<Value>) {
    for it in new {
        let is_msg = it["type"].as_str() == Some("message");
        if is_msg {
            if let Some(last) = acc.last_mut() {
                if last["type"].as_str() == Some("message") {
                    // Concatenate output_text parts across chunks.
                    let mut parts = last["content"].as_array().cloned().unwrap_or_default();
                    let mut new_parts = it["content"].as_array().cloned().unwrap_or_default();
                    let first_new_text_idx = new_parts.iter().position(|p| p["type"] == "output_text");
                    if let (Some(a), Some(bidx)) = (
                        parts.iter_mut().rev().find(|p| p["type"] == "output_text"),
                        first_new_text_idx,
                    ) {
                        let t = new_parts[bidx]["text"].as_str().unwrap_or_default().to_string();
                        let cur = a["text"].as_str().unwrap_or_default().to_string();
                        a["text"] = json!(cur + &t);
                        new_parts.remove(bidx);
                    }
                    parts.extend(new_parts);
                    last["content"] = json!(parts);
                    continue;
                }
            }
        }
        acc.push(it);
    }
}

fn record_chunk_events(items: &[Value], continues_text: bool, events: Option<&(dyn Fn(ChainEvent) + Send + Sync)>) {
    if let Some(cb) = events {
        cb(ChainEvent::ResponsesItems {
            items: items.to_vec(),
            continues_text,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn record_chunk(
    chunk: AssembledResponses,
    output_items: &mut Vec<Value>,
    resp_id: &mut String,
    created_at: &mut u64,
    last_usage: &mut Value,
    last_service_tier: &mut Value,
    ptok: &mut u64,
    ctok: &mut u64,
    input_items: &mut Vec<Value>,
    events: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
) {
    record_chunk_events(&chunk.output, false, events);
    merge_items(output_items, chunk.output.clone());
    if resp_id.is_empty() {
        *resp_id = chunk.id.clone();
        *created_at = chunk.created_at;
    }
    *ptok = (*ptok).max(chunk.input_tokens);
    *ctok += chunk.output_tokens;
    *last_usage = chunk.usage.clone();
    if !chunk.service_tier.is_null() {
        *last_service_tier = chunk.service_tier.clone();
    }
    input_items.extend(chunk.output);
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

/// Metered fallback rung: one chunk as an SSE stream (bytes flow, no idle
/// window), assembled from the stream's terminal response.completed object.
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
) -> Result<AssembledResponses, String> {
    payload["stream"] = json!(true);
    let resp = proxy::forward_cancellable(
        t, token_cache, session, browser, account, project,
        Dialect::OpenRouterResponses, step, payload, chat_session, step_index, cancelled,
    )
    .await?;
    let text = body_text(resp).await?;
    // The buffered SSE text: find the terminal response.completed event.
    let mut final_obj: Option<Value> = None;
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data: ") else { continue };
        if data == "[DONE]" {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(data) {
            if v["type"].as_str() == Some("response.completed") {
                final_obj = v.get("response").cloned();
            }
        }
    }
    let body = final_obj.ok_or("streamed chunk had no response.completed event")?;
    let usage = body.get("usage").cloned().unwrap_or_else(|| json!({}));
    Ok(AssembledResponses {
        id: body["id"].as_str().unwrap_or_default().to_string(),
        created_at: body["created_at"].as_u64().unwrap_or(0),
        output: body["output"].as_array().cloned().unwrap_or_default(),
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
pub(crate) fn prepare_input(client_payload: &Value) -> Vec<Value> {
    // Normalize the client's input to an item array (a bare string prompt is
    // legal Responses input).
    let mut input_items: Vec<Value> = match &client_payload["input"] {
        Value::String(s) => vec![json!({
            "type": "message", "role": "user",
            "content": [{"type": "input_text", "text": s}],
        })],
        Value::Array(a) => a.clone(),
        _ => Vec::new(),
    };

    // Namespace backfill (heals sessions poisoned before the verbatim-streaming
    // fix): the upstream rejects re-sent function_calls missing `namespace`
    // when the tool lives outside the default namespace ("Missing namespace for
    // function_call 'spawn_agent'", 2026-10-06). Recover the namespace from the
    // request's own tool declarations: a `namespace` field on the tool, a
    // `type:"namespace"` container holding it, or a dotted `ns.name`.
    let ns_of = {
        let mut map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if let Some(tools) = client_payload["tools"].as_array() {
            for t in tools {
                let ty = t["type"].as_str().unwrap_or_default();
                if ty == "namespace" {
                    if let Some(ns) = t["name"].as_str() {
                        for inner in t["tools"].as_array().into_iter().flatten() {
                            if let Some(n) = inner["name"].as_str() {
                                map.insert(n.to_string(), ns.to_string());
                            }
                        }
                    }
                } else if let Some(n) = t["name"].as_str() {
                    if let Some(ns) = t["namespace"].as_str() {
                        map.insert(n.to_string(), ns.to_string());
                    } else if let Some((ns, _)) = n.split_once('.') {
                        map.insert(n.rsplit('.').next().unwrap_or(n).to_string(), ns.to_string());
                    }
                }
            }
        }
        map
    };
    if !ns_of.is_empty() {
        let mut healed = 0u32;
        for it in input_items.iter_mut() {
            let ty = it["type"].as_str().unwrap_or_default();
            if (ty == "function_call" || ty == "custom_tool_call") && it.get("namespace").is_none() {
                if let Some(ns) = it["name"].as_str().and_then(|n| ns_of.get(n)) {
                    it["namespace"] = json!(ns);
                    healed += 1;
                }
            }
        }
        if healed > 0 {
            eprintln!("[nativechain] namespace backfill: healed {healed} call item(s) from tool declarations");
        }
    }

    input_items
}

// Shared preparation only. Transport and retry policies remain separate.
pub(crate) fn prepare_payload(client_payload: &Value, input: &[Value], chunk_budget: u32) -> Value {
    let mut p = client_payload.clone();
    p["model"] = json!(framer_core::convert::to_openrouter_id(
        client_payload["model"].as_str().unwrap_or_default()
    ));
    p["stream"] = json!(false);
    p["max_output_tokens"] = json!(chunk_budget.max(16)); // API floor
    p["input"] = json!(input);
    for k in ["store", "previous_response_id", "background"] {
        if let Some(obj) = p.as_object_mut() {
            obj.remove(k);
        }
    }
    p
}

/// Both native delivery paths must withhold uncertain executable tool input.
/// The caller supplies the actual prepared request budget, including its floor.
#[cfg(feature = "experimental-native")]
pub(crate) fn responses_tool_output_incomplete(body: &Value, request_budget: u32) -> bool {
    let contains_call = body["output"].as_array().is_some_and(|items| {
        items.iter().any(|item| matches!(item["type"].as_str(),
            Some("function_call") | Some("custom_tool_call")))
    });
    let hit_cap = body["status"].as_str() == Some("incomplete")
        && body["incomplete_details"]["reason"].as_str() == Some("max_output_tokens");
    contains_call && (hit_cap
        || body["usage"]["output_tokens"].as_u64().unwrap_or(0) >= u64::from(request_budget))
}

/// Explicit native Responses chain with shared typed-input/payload preparation.
/// This entry never enters browser, retry or metered fallback paths.
#[cfg(feature = "experimental-native")]
pub async fn complete_long_native(
    native: &crate::experimental_native::application::NativeInvocation<'_>,
    project: &ProjectId,
    client_payload: Value,
    cfg: &ChainConfig,
    chat_session: &str,
    events: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
) -> Result<crate::experimental_native::application::NativeChain<AssembledResponses>,
            crate::experimental_native::application::ApplicationError> {
    use crate::experimental_native::application::{self, ApplicationError, NativeChain};
    application::validate_chain(cfg)?;
    if !client_payload.is_object()
        || !(client_payload["input"].is_array() || client_payload["input"].is_string()) {
        return Err(ApplicationError::InvalidRequest("Responses requires string or array input"));
    }
    if cfg.chunk_tokens < 16 || cfg.max_total_tokens.is_some_and(|cap| cap < 16) {
        return Err(ApplicationError::InvalidRequest("Responses chunk budget must reach the API floor"));
    }
    let mut input_items = prepare_input(&client_payload);
    let mut output_items = Vec::new();
    let mut attempts = Vec::new();
    let mut resp_id = String::new();
    let mut created_at = 0u64;
    let mut last_usage = json!({});
    let mut last_service_tier = Value::Null;
    let (mut ptok, mut ctok) = (0u64, 0u64);
    let mut final_status = String::new();
    let mut final_incomplete = None;
    let fast_lane = matches!(
        client_payload["service_tier"].as_str(),
        Some("ultrafast") | Some("priority") | Some("fast")
    );
    let grow_cap = if fast_lane { 12000 } else { 3000 };
    let chunk_cap = cfg.chunk_tokens.max(if fast_lane { 4000 } else { 2500 });
    let mut chunk_tokens = if fast_lane {
        cfg.chunk_tokens.clamp(1500, grow_cap)
    } else {
        cfg.chunk_tokens.min(600)
    };
    for i in 0..cfg.max_chunks {
        let remaining = cfg.max_total_tokens
            .map(|cap| (cap as u64).saturating_sub(ctok))
            .unwrap_or(chunk_tokens as u64);
        let chunk = remaining.min(chunk_tokens as u64).min(chunk_cap as u64) as u32;
        let payload = prepare_payload(&client_payload, &input_items, chunk);
        let result = native.json(
            project, Dialect::OpenRouterResponses, payload, chat_session, i,
            |body| body.is_object() && body["id"].is_string()
                && body["output"].is_array()
                && matches!(body["status"].as_str(), Some("completed") | Some("incomplete"))
                && body.get("usage").map(|v| v.is_object()).unwrap_or(true),
        ).await;
        let (body, metadata) = match result {
            Ok(value) => value,
            Err(error) => return Err(error.with_attempts(attempts)),
        };
        let elapsed = metadata.elapsed.as_secs_f32().max(0.5);
        // Complete JSON can still contain a cut-off executable tool input.
        // A completed label is insufficient at the actual request budget.
        // Keep this entry's no-retry contract and reject before any event or
        // output publication, retaining the current and prior attempt evidence.
        if responses_tool_output_incomplete(&body, chunk.max(16)) {
            return Err(ApplicationError::InvalidResponse {
                detail: "tool output completeness is unconfirmed at the request token boundary",
                metadata,
                body: body.to_string().into_bytes(),
            }.with_attempts(attempts));
        }
        attempts.push(metadata);
        let items = body["output"].as_array().expect("validated array").clone();
        final_status = body["status"].as_str().expect("validated status").to_owned();
        final_incomplete = body.get("incomplete_details").cloned();
        if i == 0 {
            resp_id = body["id"].as_str().expect("validated id").to_owned();
            created_at = body["created_at"].as_u64().unwrap_or(0);
        }
        last_usage = body.get("usage").cloned().unwrap_or_else(|| json!({}));
        ptok = ptok.max(last_usage["input_tokens"].as_u64().unwrap_or(0));
        let out_t = last_usage["output_tokens"].as_u64().unwrap_or(0);
        ctok = ctok.saturating_add(out_t);
        if !body["service_tier"].is_null() {
            last_service_tier = body["service_tier"].clone();
        }
        let tps = out_t as f32 / elapsed;
        if tps > 1.0 {
            chunk_tokens = ((tps * 45.0) as u32).clamp(400, grow_cap);
        }
        let hit_cap = final_status == "incomplete"
            && body["incomplete_details"]["reason"].as_str() == Some("max_output_tokens");
        let text_output = items.last()
            .is_some_and(|item| item["type"].as_str() == Some("message"))
            && items.iter().all(|item| {
                matches!(item["type"].as_str(), Some("message") | Some("reasoning"))
            });
        let remaining = cfg.max_total_tokens
            .map(|cap| (cap as u64).saturating_sub(ctok));
        let continues = hit_cap && text_output && i + 1 < cfg.max_chunks
            && !remaining.is_some_and(|left| left < 16)
            && !cfg.continue_prompt.is_empty();
        record_chunk_events(&items, continues, events);
        merge_items(&mut output_items, items.clone());
        if !continues { break; }
        // Only the explicit caller configuration supplies a continuation.
        input_items.extend(items);
        input_items.push(json!({
            "type": "message", "role": "user",
            "content": [{"type": "input_text", "text": cfg.continue_prompt}],
        }));
    }
    Ok(NativeChain {
        response: AssembledResponses {
            id: resp_id, created_at, output: output_items, status: final_status,
            incomplete_details: final_incomplete, usage: last_usage,
            service_tier: last_service_tier, input_tokens: ptok, output_tokens: ctok,
            chunks: attempts.len() as u32, metered_chunks: 0,
        },
        attempts,
    })
}
