//! Cutoff response renderers: Assembled outcomes -> client dialect shapes
//! (SSE, responses JSON/frames, chat chunks/completions). Pure functions,
//! no I/O. Pure move out of custom.rs (no behavior change).

use super::custom::CutoffOutcome;
use axum::response::{IntoResponse, Response};

pub(super) fn sse_response(assembled: &framer_custom::Assembled, req_id: &str, refunded: u32) -> Response {
    // Frame contract: text+legs are the stable fields every client reads.
    // outcome/total_debit/legs-detail/req_id are additive diagnostics so a
    // billed, short, or raced run is diagnosable from the response alone
    // (mirrors the [custom-route] daemon log lines, same req_id).
    let event = format!(
        "data: {}\n\n",
        serde_json::json!({
            "text": assembled.text,
            "legs": assembled.legs.len(),
            "outcome": format!("{:?}", assembled.outcome),
            "total_debit": assembled.total_debit,
            // Deterministic progress + the compaction input: per-leg captured
            // chars and an estimated output-token count. The abort lands
            // BEFORE the usage frame by design, so real token counts never
            // arrive -- the estimate (chars/4, the standard heuristic) is
            // marked as such, never presented as metered truth.
            "estimated_output_tokens": (assembled.text.chars().count() / 4).max(1),
            "token_estimate": "estimated", // honest marker: never metered truth
            "legs_detail": assembled.legs.iter().map(|l| serde_json::json!({
                "raced": l.raced, "debited": l.debited,
                "stop_used": l.stop_used, "debit": l.debit, "events": l.events,
                "chars": l.text.chars().count(),
            })).collect::<Vec<_>>(),
            "req_id": req_id,
            // Settles this chain reversed via /ai/v3/refund/ (0 on a clean run).
            "turns_refunded": refunded,
        })
    );
    ([("content-type", "text/event-stream")], event).into_response()
}

pub(super) fn render_responses(out: &CutoffOutcome, model: &str, stream: bool) -> Response {
    let resp_id = format!("resp_{}", uuid::Uuid::new_v4().simple());
    let created = framer_core::rotation::now_ms() / 1000;
    let mut output: Vec<serde_json::Value> = Vec::new();
    let text = &out.assembled.text;
    if !text.is_empty() {
        output.push(serde_json::json!({
            "id": format!("msg_{}", uuid::Uuid::new_v4().simple()),
            "type": "message",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": text}],
        }));
    }
    for tc in &out.assembled.tool_calls {
        output.push(serde_json::json!({
            "id": format!("fc_{}", uuid::Uuid::new_v4().simple()),
            "type": "function_call",
            "call_id": tc["id"].as_str().unwrap_or(""),
            "name": tc["name"].as_str().unwrap_or(""),
            "arguments": tc["arguments"].as_str().unwrap_or(""),
            "status": "completed",
        }));
    }
    let completion_tokens = text.chars().count() / 4;
    let usage = serde_json::json!({
        "input_tokens": 0,
        "output_tokens": completion_tokens,
        "total_tokens": completion_tokens,
    });
    if !stream {
        return axum::Json(serde_json::json!({
            "id": resp_id,
            "object": "response",
            "created_at": created,
            "status": "completed",
            "model": model,
            "output": output,
            "usage": usage,
            "x-framerize-usage": "estimated",
        }))
        .into_response();
    }
    let body = responses_frames(out, model);
    return ([("content-type", "text/event-stream")], body).into_response();
}

pub(super) fn responses_frames(out: &CutoffOutcome, model: &str) -> String {
    let resp_id = format!("resp_{}", uuid::Uuid::new_v4().simple());
    let created = framer_core::rotation::now_ms() / 1000;
    let mut output: Vec<serde_json::Value> = Vec::new();
    let text = &out.assembled.text;
    if !text.is_empty() {
        output.push(serde_json::json!({
            "id": format!("msg_{}", uuid::Uuid::new_v4().simple()),
            "type": "message",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": text}],
        }));
    }
    for tc in &out.assembled.tool_calls {
        output.push(serde_json::json!({
            "id": format!("fc_{}", uuid::Uuid::new_v4().simple()),
            "type": "function_call",
            "call_id": tc["id"].as_str().unwrap_or(""),
            "name": tc["name"].as_str().unwrap_or(""),
            "arguments": tc["arguments"].as_str().unwrap_or(""),
            "status": "completed",
        }));
    }
    let completion_tokens = text.chars().count() / 4;
    let usage = serde_json::json!({
        "input_tokens": 0,
        "output_tokens": completion_tokens,
        "total_tokens": completion_tokens,
    });
    let mut body = String::new();
    body.push_str(&format!(
        "data: {}\n\n",
        serde_json::json!({"type": "response.created", "response": {"id": resp_id, "object": "response", "created_at": created, "status": "in_progress", "model": model, "output": []}})
    ));
    for (i, item) in output.iter().enumerate() {
        body.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({"type": "response.output_item.added", "output_index": i, "item": item})
        ));
        if item["type"].as_str() == Some("message") {
            let full = item["content"][0]["text"].as_str().unwrap_or("");
            body.push_str(&format!(
                "data: {}\n\n",
                serde_json::json!({"type": "response.output_text.delta", "output_index": i, "content_index": 0, "delta": full})
            ));
        }
        body.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({"type": "response.output_item.done", "output_index": i, "item": item})
        ));
    }
    body.push_str(&format!(
        "data: {}\n\n",
        serde_json::json!({
            "type": "response.completed",
            "response": {"id": resp_id, "object": "response", "created_at": created, "status": "completed", "model": model, "output": output, "usage": usage},
        })
    ));
    body
}

pub(super) fn chat_chunk(id: &str, created: u64, model: &str, delta: serde_json::Value) -> String {
    format!(
        "data: {}\n\n",
        serde_json::json!({
            "id": id, "object": "chat.completion.chunk", "created": created, "model": model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": null}],
        })
    )
}

pub(super) fn chat_completion_json(
    out: &CutoffOutcome,
    model: &str,
    include_usage: bool,
    prompt_chars: usize,
) -> Response {
    let text = &out.assembled.text;
    let completion_tokens = text.chars().count() / 4;
    let prompt_tokens = prompt_chars / 4;
    let has_tools = !out.assembled.tool_calls.is_empty();
    let message = if has_tools {
        serde_json::json!({"role": "assistant", "content": if text.is_empty() { serde_json::Value::Null } else { serde_json::json!(text) }, "tool_calls": framer_core::convert::tool_calls_openai(&out.assembled.tool_calls)})
    } else {
        serde_json::json!({"role": "assistant", "content": text})
    };
    let mut body = serde_json::json!({
        "id": format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()),
        "object": "chat.completion",
        "created": framer_core::rotation::now_ms() / 1000,
        "model": model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": if has_tools { "tool_calls" } else { "stop" },
        }],
    });
    if include_usage {
        body["usage"] = serde_json::json!({
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens,
        });
        body["x-framerize-usage"] = serde_json::Value::String("estimated".into());
    }
    axum::Json(body).into_response()
}
