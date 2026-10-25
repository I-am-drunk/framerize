use serde_json::json;
use serde_json::Value;
use framer_core::ids::ProjectId;
use framer_core::port::ChainConfig;
use framer_core::convert::ChainEvent;
use framer_core::convert::AssembledResponse;
use framer_core::convert::AssembledResponses;
use framer_api::proxy::Dialect;
use framer_api::protocol::responses::*;
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
pub async fn complete_long_native(
    native: &crate::request::invocation::NativeInvocation<'_>,
    project: &ProjectId,
    client_payload: Value,
    cfg: &ChainConfig,
    chat_session: &str,
    events: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
) -> Result<crate::request::invocation::NativeChain<AssembledResponses>,
            crate::request::invocation::ApplicationError> {
    use crate::request::invocation as application;
    use crate::request::invocation::ApplicationError;
    use crate::request::invocation::NativeChain;
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
