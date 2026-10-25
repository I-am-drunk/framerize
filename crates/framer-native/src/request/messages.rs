use serde_json::json;
use serde_json::Value;
use framer_core::ids::ProjectId;
use framer_core::port::ChainConfig;
use framer_core::convert::ChainEvent;
use framer_core::convert::AssembledResponse;
use framer_core::convert::AssembledResponses;
use framer_api::proxy::Dialect;
use framer_api::protocol::messages::*;
pub async fn complete_long_native(
    native: &crate::request::invocation::NativeInvocation<'_>,
    project: &ProjectId,
    anthro_payload: Value,
    cfg: &ChainConfig,
    chat_session: &str,
    events: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
) -> Result<crate::request::invocation::NativeChain<AssembledResponse>,
            crate::request::invocation::ApplicationError> {
    use crate::request::invocation as application;
    use crate::request::invocation::ApplicationError;
    use crate::request::invocation::NativeChain;
    application::validate_chain(cfg)?;
    if !anthro_payload.is_object() || !anthro_payload["messages"].is_array() {
        return Err(ApplicationError::InvalidRequest("Messages requires an array history"));
    }
    let mut messages = anthro_payload["messages"].clone();
    let mut blocks = Vec::new();
    let mut attempts = Vec::new();
    let mut msg_id = String::new();
    let mut stop_reason = String::new();
    let mut stop_sequence = None;
    let mut last_usage = json!({});
    let (mut ptok, mut ctok) = (0u64, 0u64);
    let mut chunk_tokens = cfg.chunk_tokens.min(600);
    for i in 0..cfg.max_chunks {
        let remaining = cfg.max_total_tokens
            .map(|cap| (cap as u64).saturating_sub(ctok))
            .unwrap_or(chunk_tokens as u64);
        let chunk = remaining.min(chunk_tokens as u64) as u32;
        let payload = prepare_payload(&anthro_payload, &messages, chunk, Dialect::OpenRouterMessages);
        let result = native.json(
            project, Dialect::OpenRouterMessages, payload, chat_session, i,
            |body| body.is_object() && body["id"].is_string()
                && body["content"].is_array() && body["stop_reason"].is_string()
                && body.get("usage").map(|v| v.is_object()).unwrap_or(true),
        ).await;
        let (body, metadata) = match result {
            Ok(value) => value,
            Err(error) => return Err(error.with_attempts(attempts)),
        };
        let elapsed = metadata.elapsed.as_secs_f32().max(0.5);
        attempts.push(metadata);
        let chunk_blocks = body["content"].as_array().expect("validated array").clone();
        stop_reason = body["stop_reason"].as_str().expect("validated stop").to_owned();
        stop_sequence = body["stop_sequence"].as_str().map(str::to_owned);
        if i == 0 {
            msg_id = body["id"].as_str().expect("validated id").to_owned();
        }
        last_usage = body.get("usage").cloned().unwrap_or_else(|| json!({}));
        ptok = ptok.max(last_usage["input_tokens"].as_u64().unwrap_or(0));
        let out_t = last_usage["output_tokens"].as_u64().unwrap_or(0);
        ctok = ctok.saturating_add(out_t);
        let tps = out_t as f32 / elapsed;
        if tps > 1.0 {
            chunk_tokens = ((tps * 45.0) as u32).clamp(400, 3000);
        }
        merge_blocks(&mut blocks, chunk_blocks.clone());
        if let Some(cb) = events {
            cb(ChainEvent::Chunk(chunk_blocks.clone()));
        }
        let budget_done = cfg.max_total_tokens
            .map(|cap| ctok >= cap as u64).unwrap_or(false);
        // Incomplete tools or other non-text blocks are delivered verbatim.
        // Never drop them, invent repair history or retry the same operation.
        let text_only = !chunk_blocks.is_empty()
            && chunk_blocks.iter().all(|b| b["type"].as_str() == Some("text"));
        if stop_reason != "max_tokens" || budget_done || i + 1 == cfg.max_chunks
            || !text_only || cfg.continue_prompt.is_empty() {
            break;
        }
        push_continuation(&mut messages, &chunk_blocks, cfg);
    }
    last_usage["output_tokens"] = json!(ctok);
    last_usage["input_tokens"] = json!(ptok);
    Ok(NativeChain {
        response: AssembledResponse {
            id: msg_id, content: blocks, stop_reason, stop_sequence,
            usage: last_usage, input_tokens: ptok, output_tokens: ctok,
            chunks: attempts.len() as u32, metered_chunks: 0,
        },
        attempts,
    })
}
