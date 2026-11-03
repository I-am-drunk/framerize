use super::StreamedChunk;
use serde_json::{json, Value};

#[cfg(test)]
#[path = "messages_stream_tests.rs"]
mod tests;

fn require_active(completion: &Completion) -> Result<(), String> {
    if !completion.started || completion.stopped {
        return Err("streamed Messages response has an event outside its message".into());
    }
    Ok(())
}

fn block_index(event: &Value) -> Result<usize, String> {
    event["index"].as_u64().and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| "streamed Messages response has an invalid block index".into())
}

fn open_block(event: &Value, completion: &Completion) -> Result<usize, String> {
    require_active(completion)?;
    let index = block_index(event)?;
    if completion.open_blocks.get(index) != Some(&true) {
        return Err("streamed Messages response refers to a missing or closed block".into());
    }
    Ok(index)
}

fn append_delta(block: &mut Value, kind: &str, field: &str, delta: &Value) -> Result<(), String> {
    let actual = block["type"].as_str();
    if actual != Some(kind) && !(kind == "tool_use" && actual == Some("server_tool_use")) {
        return Err("streamed Messages response has a delta for the wrong block type".into());
    }
    let text = delta.as_str().ok_or("streamed Messages response has a non-text delta")?;
    let mut joined = match block.get(field) {
        None => String::new(),
        Some(Value::String(value)) => value.clone(),
        _ => return Err("streamed Messages response has a non-text block field".into()),
    };
    joined.push_str(text);
    block[field] = json!(joined);
    Ok(())
}

fn replace_signature(block: &mut Value, signature: &Value) -> Result<(), String> {
    if block["type"] != "thinking" {
        return Err("streamed Messages response has a delta for the wrong block type".into());
    }
    let signature = signature.as_str().ok_or("streamed Messages response has a non-text delta")?;
    if block.get("signature").is_some_and(|value| !value.is_string()) {
        return Err("streamed Messages response has a non-text block field".into());
    }
    // Signatures are complete opaque values, not incremental text fragments.
    block["signature"] = json!(signature);
    Ok(())
}

fn append_citation(block: &mut Value, citation: &Value) -> Result<(), String> {
    if block["type"] != "text" || !citation.is_object() {
        return Err("streamed Messages response has an invalid citation delta".into());
    }
    if block.get("citations").map_or(true, Value::is_null) {
        block["citations"] = json!([]);
    }
    block["citations"].as_array_mut()
        .ok_or("streamed Messages response has invalid citations")?
        .push(citation.clone());
    Ok(())
}

fn merge_usage(event: &Value, usage: &mut Value, partial: bool) -> Result<(), String> {
    if let Some(update) = event.get("usage") {
        let mut update = update.clone();
        if partial {
            if let Some(fields) = update.as_object_mut() {
                // These nullable delta fields mean no update in the Messages SDK.
                for field in ["input_tokens", "cache_creation_input_tokens",
                    "cache_read_input_tokens", "server_tool_use", "output_tokens_details"] {
                    if fields.get(field).is_some_and(Value::is_null) { fields.remove(field); }
                }
            }
        }
        for (field, value) in super::buffered::validate_usage(&update)? {
            // Event counters are cumulative snapshots, not increments.
            usage[field] = value.clone();
        }
    }
    Ok(())
}

#[derive(Default)]
struct Completion {
    started: bool,
    stopped: bool,
    open_blocks: Vec<bool>,
}

// SSE accepts LF, CRLF and lone CR. A CRLF pair is one line ending.
fn sse_lines(mut text: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        if text.is_empty() { return None; }
        let end = text.find(['\r', '\n']).unwrap_or(text.len());
        let (line, rest) = text.split_at(end);
        text = rest.strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix('\r'))
            .or_else(|| rest.strip_prefix('\n'))
            .unwrap_or_default();
        Some(line)
    })
}

pub(super) fn parse(text: &str) -> Result<StreamedChunk, String> {
    let mut id = String::new();
    let mut blocks: Vec<Value> = Vec::new();
    let mut stop_reason = String::new();
    let mut stop_sequence: Option<String> = None;
    let mut stop_details: Option<Value> = None;
    let mut usage = json!({});
    let mut current_data = String::new();
    let mut current_event = String::new();
    let mut completion = Completion::default();

    #[allow(clippy::too_many_arguments)]
    fn handle_event(
        data: &str,
        event: &str,
        id: &mut String,
        blocks: &mut Vec<Value>,
        stop_reason: &mut String,
        stop_sequence: &mut Option<String>,
        stop_details: &mut Option<Value>,
        usage: &mut Value,
        completion: &mut Completion,
    ) -> Result<(), String> {
        if data.is_empty() {
            if event == "error" {
                return Err("streamed Messages upstream error event contained no data".into());
            }
            return Ok(());
        }
        let v: Value = serde_json::from_str(data).map_err(|_| {
            if event == "error" { "streamed Messages upstream error event has malformed JSON" }
            else { "streamed Messages response has malformed event JSON" }
        })?;
        if event == "error" || v["type"] == "error" {
            return Err(super::buffered::upstream_error(&v));
        }
        match v["type"].as_str() {
            Some("message_start") => {
                if completion.started || completion.stopped {
                    return Err("streamed Messages response repeated message_start".into());
                }
                *id = v["message"]["id"].as_str().filter(|id| !id.is_empty())
                    .ok_or("streamed Messages response has no message id")?.to_string();
                super::buffered::validate_message_metadata(&v["message"])?;
                *stop_details = super::buffered::stop_details(&v["message"])?;
                completion.started = true;
                merge_usage(&v["message"], usage, false)?;
            }
            Some("content_block_start") => {
                require_active(completion)?;
                let idx = block_index(&v)?;
                if idx != blocks.len() || !stop_reason.is_empty() {
                    return Err("streamed Messages response has an out-of-order block".into());
                }
                if !v["content_block"].is_object() || !v["content_block"]["type"].is_string() {
                    return Err("streamed Messages response has an invalid block".into());
                }
                blocks.push(v["content_block"].clone());
                completion.open_blocks.push(true);
            }
            Some("content_block_delta") => {
                let idx = open_block(&v, completion)?;
                match v["delta"]["type"].as_str() {
                    Some("text_delta") => {
                        append_delta(&mut blocks[idx], "text", "text", &v["delta"]["text"])?;
                    }
                    Some("citations_delta") => {
                        append_citation(&mut blocks[idx], &v["delta"]["citation"])?;
                    }
                    Some("signature_delta") => {
                        replace_signature(&mut blocks[idx], &v["delta"]["signature"])?;
                    }
                    Some("thinking_delta") => {
                        append_delta(&mut blocks[idx], "thinking", "thinking", &v["delta"]["thinking"])?;
                    }
                    Some("input_json_delta") => {
                        append_delta(&mut blocks[idx], "tool_use", "_partial_json", &v["delta"]["partial_json"])?;
                    }
                    None => return Err("streamed Messages response has no delta type".into()),
                    _ => {}
                }
            }
            Some("content_block_stop") => {
                let idx = open_block(&v, completion)?;
                completion.open_blocks[idx] = false;
            }
            Some("message_delta") => {
                require_active(completion)?;
                let delta = v["delta"].as_object()
                    .ok_or("streamed Messages response has an invalid message delta")?;
                super::buffered::validate_stop_sequence(&v["delta"])?;
                if delta.contains_key("stop_details") {
                    *stop_details = super::buffered::stop_details(&v["delta"])?;
                }
                if let Some(reason) = delta.get("stop_reason").filter(|reason| !reason.is_null()) {
                    *stop_reason = super::buffered::terminal_stop_reason(reason)?.to_string();
                    *stop_sequence = v["delta"]["stop_sequence"].as_str().map(str::to_string);
                }
                merge_usage(&v, usage, true)?;
            }
            Some("message_stop") => {
                require_active(completion)?;
                if stop_reason.is_empty() || completion.open_blocks.iter().any(|open| *open) {
                    return Err("streamed Messages response stopped before content completed".into());
                }
                completion.stopped = true;
            }
            None => return Err("streamed Messages response has no event type".into()),
            _ => {}
        }
        Ok(())
    }

    for line in sse_lines(text) {
        if let Some(d) = line.strip_prefix("data:") {
            let d = d.trim_start();
            if !current_data.is_empty() {
                current_data.push('\n');
            }
            current_data.push_str(d);
        } else if let Some(event) = line.strip_prefix("event:") {
            current_event = event.strip_prefix(' ').unwrap_or(event).to_string();
        } else if line == "event" {
            current_event.clear();
        } else if line.trim().is_empty() {
            handle_event(&current_data, &current_event, &mut id, &mut blocks, &mut stop_reason,
                &mut stop_sequence, &mut stop_details, &mut usage, &mut completion)?;
            current_data.clear();
            current_event.clear();
        }
    }
    if !current_data.is_empty() || !current_event.is_empty() {
        handle_event(&current_data, &current_event, &mut id, &mut blocks, &mut stop_reason,
            &mut stop_sequence, &mut stop_details, &mut usage, &mut completion)?;
    }

    if !completion.stopped {
        return Err("streamed Messages response ended without message_stop".into());
    }

    // Validate tool input only after the complete message has been observed.
    for b in blocks.iter_mut() {
        if matches!(b["type"].as_str(), Some("tool_use" | "server_tool_use")) {
            if stop_reason == "max_tokens" {
                return Err("streamed Messages response contains an incomplete tool call".into());
            }
            let raw = b["_partial_json"].as_str().unwrap_or_default().to_string();
            // Keep the input from content_block_start when no deltas arrived.
            if !raw.is_empty() {
                b["input"] = serde_json::from_str::<Value>(&raw)
                    .map_err(|_| "streamed Messages response has invalid tool input JSON")?;
            }
            if !b["input"].is_object() {
                return Err("streamed Messages response has non-object tool input".into());
            }
            if let Some(obj) = b.as_object_mut() {
                obj.remove("_partial_json");
            }
        }
    }

    super::buffered::validate_blocks(&blocks, &stop_reason)?;
    let ptok = usage["input_tokens"].as_u64().unwrap_or(0);
    let ctok = usage["output_tokens"].as_u64().unwrap_or(0);
    usage["input_tokens"] = json!(ptok);
    usage["output_tokens"] = json!(ctok);
    Ok(StreamedChunk {
        id,
        content: blocks,
        stop_reason,
        stop_sequence,
        stop_details,
        usage,
        input_tokens: ptok,
        output_tokens: ctok,
    })
}
