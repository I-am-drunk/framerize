//! Agent inference: POST /ai/v3/openai/responses/ — the editor Agent panel's
//! message call. One user turn per call; conversation state is server-side keyed
//! by sessionId (client-minted). Response: SSE stream of OpenAI Responses-API
//! events; remaining credits in the X-Framer-AI-Usage header.
//!
//! Wire body (VERIFIED 200 on 2026-10-01 — the `request` field is REQUIRED;
//! the server 400s "request is required" without it and 400s "model pricing is
//! not configured" for unknown model ids). Model ids have NO provider prefix here:
//! "gpt-6-sol", not "openai/gpt-6-sol".
//!   {projectId, featureCategory:"agents", featureName:"agents", step:"message",
//!    sessionId, stepIndex, turnId, userPrompt, castleToken,
//!    request: {model, input, stream: true}}
//! Verified SSE vocabulary: response.created, response.in_progress,
//! response.output_item.added, response.content_part.added,
//! response.output_text.delta, response.output_text.done, response.content_part.done,
//! response.output_item.done, response.completed (full response object incl. usage).
//! Verified header: X-Framer-AI-Usage: remaining=484 (credits decrement per run).
//!
//! The generic proxy envelope for the OTHER dialect routes is Gq's shape:
//!   {projectId, ...meta, castleToken, request: <native payload>}
//! See reference/analysis/framer-ai-endpoints.md for the full route table.

use crate::auth::{self, AiToken};
use crate::credits;
use crate::transport::Transport;
use framer_core::ids::{AccountId, ProjectId};
use framer_core::pipeline::{InferenceOutcome, InferenceRequest, InferenceResult, Usage};
use framer_core::port::{BrowserBridge, Session};
use serde_json::json;
use std::sync::Mutex;

#[cfg(test)]
#[path = "inference_tests.rs"]
mod tests;

/// Mint a chat-session / turn id in the app's style (~10 chars, mixed-case alnum).
/// pub(crate) wrapper used by proxy.rs.
pub fn mint_id_pub() -> String {
    mint_id()
}

fn mint_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static C: AtomicU64 = AtomicU64::new(0);
    let mut n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        ^ (std::process::id() as u128) << 64
        ^ (C.fetch_add(1, Ordering::Relaxed) as u128) << 96;
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    // 9 base62 chars — matches the real client's turnId/sessionId shape
    // (verified capture: "turnId":"LHXgk7pUf"). 10-char ids stood out.
    let mut out = String::with_capacity(9);
    for _ in 0..9 {
        out.push(ALPHA[(n % 62) as usize] as char);
        n /= 62;
    }
    out
}

fn build_body(req: &InferenceRequest, chat_session: &str, project: &str) -> serde_json::Value {
    let model = req.model.as_deref().unwrap_or(crate::models::DEFAULT_AGENT_MODEL);
    json!({
        "projectId": project,
        "featureCategory": "agents",
        "featureName": "agents",
        "step": "message",
        "sessionId": chat_session,
        "stepIndex": 0, // TODO: increments per turn within a session; 0 accepted on turn 1
        "turnId": mint_id(),
        "userPrompt": req.prompt,
        // castleToken is added in-page at fetch time (fresh mint, same env).
        // The actual Responses payload — REQUIRED (server 400s without it).
        "request": {
            "model": model,
            "input": req.prompt,
            "stream": true,
        },
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    t: &Transport,
    token_cache: &Mutex<Vec<(AccountId, AiToken)>>,
    session: &Session,
    browser: &dyn BrowserBridge,
    account: &AccountId,
    project: &ProjectId,
    req: &InferenceRequest,
) -> Result<InferenceResult, String> {
    let token = cached_token(t, token_cache, session, account).await?;
    let chat_session = req.chat_session.clone().unwrap_or_else(mint_id);

    match send(browser, session, account, &token, req, &chat_session, project).await? {
        Send::Done(outcome) => Ok(InferenceResult::Completed(outcome)),
        Send::Exhausted => Ok(InferenceResult::Exhausted {
            reason: "http-402".into(),
        }),
        // 401 = the cached JWT died mid-flight: drop it, remint, retry exactly once.
        Send::Unauthorized => {
            auth::invalidate_ai_token(session, &token);
            if let Ok(mut guard) = token_cache.lock() {
                guard.retain(|(id, _)| id != account);
            }
            let fresh = cached_token(t, token_cache, session, account).await?;
            match send(browser, session, account, &fresh, req, &chat_session, project).await? {
                Send::Done(outcome) => Ok(InferenceResult::Completed(outcome)),
                Send::Exhausted => Ok(InferenceResult::Exhausted {
                    reason: "http-402".into(),
                }),
                Send::Unauthorized => Err("unauthorized after token refresh".into()),
            }
        }
    }
}

enum Send {
    Done(InferenceOutcome),
    Exhausted,
    Unauthorized,
}

async fn cached_token(
    t: &Transport,
    cache: &Mutex<Vec<(AccountId, AiToken)>>,
    session: &Session,
    account: &AccountId,
) -> Result<AiToken, String> {
    // Per-account cache (2026-10-05): the old SINGLE global slot meant every
    // account rotation invalidated the previous chain's JWT and forced a
    // remint at /auth/ai/access-token — ~1 mint per chain. Under a 300-chain
    // burst that thrash trips the auth endpoint's sustained-rate limiter
    // (measured: 120/120 mints -> 403), which transport classified as Castle
    // "risk-refused" (elapsed=0), poisoning account health with phantom
    // refusals. Per-account caching cuts mints ~99%.
    if let Ok(guard) = cache.lock() {
        if let Some((_, tok)) = guard.iter().find(|(id, _)| id == account) {
            if tok.usable_for(session) {
                return Ok(tok.clone());
            }
        }
    }
    let tok = auth::ai_access_token(t, session).await?;
    if let Ok(mut guard) = cache.lock() {
        guard.retain(|(id, _)| id != account);
        guard.push((*account, tok.clone()));
    }
    Ok(tok)
}

/// Send one agent turn IN-PAGE from the account's browser (transport must
/// share the Castle mint's environment — verified quadrant matrix 2026-10-02;
/// the reqwest variant of this exact envelope gets risk-refused).
async fn send(
    browser: &dyn BrowserBridge,
    session: &Session,
    account: &AccountId,
    token: &AiToken,
    req: &InferenceRequest,
    chat_session: &str,
    project: &ProjectId,
) -> Result<Send, String> {
    browser.seed_session(account, &session.cookie_header).await?;
    let sent_at = std::time::Instant::now();
    let (status, usage_hdr, text) = browser
        .ai_post(
            account,
            crate::routes::by_id("direct-responses").map(|r| r.path).unwrap_or("/ai/v3/openai/responses/"),
            &token.bearer,
            &build_body(req, chat_session, &project.0),
        )
        .await?;
    let elapsed = sent_at.elapsed();
    match status {
        200..=299 => {}
        401 => return Ok(Send::Unauthorized),
        402 => return Ok(Send::Exhausted),
        // See classify.rs: a 403 on an OpenRouter passthrough route is only
        // account health when Framer's own edge refused it.
        403 => {
            let r = crate::classify::classify_403(&text, elapsed.as_millis() as u64);
            return Err(r.as_error(crate::routes::by_id("direct-responses").map(|r| r.path).unwrap_or("/ai/v3/openai/responses/"), &text));
        }
        s => {
            let clipped = crate::error_body::prefix(&text, 500);
            return Err(format!("retryable {s} on {}: {clipped}", crate::routes::by_id("direct-responses").map(|r| r.path).unwrap_or("/ai/v3/openai/responses/")));
        }
    }

    let remaining = usage_hdr
        .as_deref()
        .and_then(credits::parse_usage_header);

    let (answer, usage) = parse_sse(&text)?;

    Ok(Send::Done(InferenceOutcome {
        text: answer,
        usage,
        chat_session: chat_session.to_string(),
        credits_remaining: remaining,
    }))
}

// Completion carries the authoritative output; older replies may omit it and
// must keep using the text deltas. An empty output array is still authoritative.
fn completed_text(response: &serde_json::Value) -> Result<Option<String>, &'static str> {
    if !response.is_object() {
        return Err("invalid response object");
    }
    if response.get("status").is_some_and(|status| status.as_str() != Some("completed")) {
        return Err("invalid completed status");
    }
    let output = response.get("output")
        .map(|output| output.as_array().ok_or("invalid output array")).transpose()?;
    let mut text = String::new();
    for item in output.into_iter().flatten() {
        if !item.get("type").and_then(serde_json::Value::as_str)
            .is_some_and(|kind| !kind.is_empty()) {
            return Err("invalid output item");
        }
        if item["type"] != "message" {
            continue;
        }
        // Direct results have no partial-status field; unfinished text cannot
        // be returned as a completed answer even if its parent is completed.
        if item.get("status").is_some_and(|status| status.as_str() != Some("completed")) {
            return Err("invalid message status");
        }
        let content = item["content"].as_array().ok_or("invalid message content")?;
        for part in content {
            let kind = part.get("type").and_then(serde_json::Value::as_str)
                .filter(|kind| !kind.is_empty()).ok_or("invalid message part")?;
            match kind {
                "output_text" => {
                    text.push_str(part["text"].as_str().ok_or("invalid output text")?);
                }
                "refusal" if !part["refusal"].is_string() => {
                    return Err("invalid refusal");
                }
                _ => {}
            }
        }
    }
    if output.into_iter().flatten().any(|item| {
        item["type"] == "message"
            && item.get("role").is_some_and(|role| role.as_str() != Some("assistant"))
    }) {
        // Content errors precede usage errors, which precede role errors.
        completed_usage(response)?;
        return Err("invalid message role");
    }
    if !response["error"].is_null() {
        completed_usage(response)?;
        return Err("non-null response error");
    }
    Ok(output.map(|_| text))
}

fn completed_usage(response: &serde_json::Value) -> Result<Usage, &'static str> {
    let usage = &response["usage"];
    if !usage.is_null() && !usage.is_object() {
        return Err("invalid usage object");
    }
    if ["input_tokens", "output_tokens"].iter().any(|field| {
        usage.get(*field).is_some_and(|count| count.as_u64().is_none())
    }) {
        return Err("invalid usage count");
    }
    Ok(Usage {
        input_tokens: usage["input_tokens"].as_u64().unwrap_or(0),
        output_tokens: usage["output_tokens"].as_u64().unwrap_or(0),
    })
}

/// Parse the SSE stream (multi-line `data:` safe). Verified event vocabulary
/// (2026-10-01): response.created, response.in_progress, response.output_item.added,
/// response.content_part.added, response.output_text.delta, response.output_text.done,
/// response.content_part.done, response.output_item.done, response.completed.
/// `response.completed` carries the full response object including
/// usage.{input_tokens,output_tokens,total_tokens}.
/// HTTP success and an SSE terminator alone do not establish provider completion.
fn parse_sse(body: &str) -> Result<(String, Usage), String> {
    let mut out = String::new();
    let mut invalid_text_delta = false;
    let mut final_text = None;
    let mut completed_delta_len = 0;
    let mut usage = Usage::default();
    let mut completed = false;
    let mut data_lines: Vec<String> = Vec::new();
    let mut event_name = "";
    // SSE permits CR alone; a CRLF pair must still end just one line.
    let lines = body.split_terminator('\n').flat_map(|line| {
        line.strip_suffix('\r').unwrap_or(line).split('\r')
    });
    for line in lines.chain(std::iter::once("")) {
        if let Some(name) = line.strip_prefix("event:") {
            event_name = name.strip_prefix(' ').unwrap_or(name);
            continue;
        }
        if line == "event" {
            event_name = "";
            continue;
        }
        match line.strip_prefix("data:").or_else(|| (line == "data").then_some("")) {
            Some(d) => data_lines.push(d.trim_start_matches(' ').to_string()),
            None if line.is_empty() => {
                let frame_name = std::mem::take(&mut event_name);
                if matches!(frame_name, "error" | "response.failed" | "response.incomplete") {
                    return Err(format!("inference stream ended with {frame_name}"));
                }
                let named_terminal = frame_name == "response.completed";
                if data_lines.is_empty() && !named_terminal {
                    continue;
                }
                let joined = data_lines.join("\n");
                data_lines.clear();
                let parsed = serde_json::from_str::<serde_json::Value>(&joined);
                if frame_name == "response.output_text.delta"
                    && parsed.as_ref().ok().and_then(|v| v["type"].as_str()) != Some(frame_name)
                {
                    invalid_text_delta = true;
                }
                if joined == "[DONE]" && !named_terminal {
                    continue;
                }
                if named_terminal && parsed.as_ref().ok().and_then(|v| v["type"].as_str()) != Some(frame_name) {
                    return Err("invalid inference completion: invalid named terminal event".into());
                }
                if let Ok(ev) = parsed {
                    match ev["type"].as_str() {
                        Some("response.output_text.delta") => {
                            if let Some(d) = ev["delta"].as_str() {
                                out.push_str(d);
                            } else {
                                invalid_text_delta = true;
                            }
                        }
                        Some("response.completed") => {
                            final_text = completed_text(&ev["response"])
                                .map_err(|reason| format!("invalid inference completion: {reason}"))?;
                            completed_delta_len = out.len();
                            completed = true;
                            usage = completed_usage(&ev["response"])
                                .map_err(|reason| format!("invalid inference completion: {reason}"))?;
                        }
                        Some(kind @ ("response.failed" | "response.incomplete" | "error")) => {
                            return Err(format!("inference stream ended with {kind}"));
                        }
                        _ => {}
                    }
                }
            }
            None => {}
        }
    }
    if completed {
        // Only legacy replies without authoritative output rely on the deltas.
        if final_text.is_none() && invalid_text_delta {
            return Err("invalid inference completion: invalid output text delta".into());
        }
        // A legacy answer ends at its last completion, not at a trailing delta.
        out.truncate(completed_delta_len);
        Ok((final_text.unwrap_or(out), usage))
    } else {
        Err("inference stream ended before response.completed".into())
    }
}
