use serde_json::{json, Map, Value};

#[cfg(test)]
#[path = "messages_json_tests.rs"]
mod tests;

fn nonempty_text(value: &Value) -> bool {
    value.as_str().is_some_and(|text| !text.trim().is_empty())
}

pub(super) fn decode(body: &Value) -> Result<(Vec<Value>, String, Value), String> {
    if !body.is_object() {
        return Err("Messages response is not an object".into());
    }
    if body["type"] == "error" || body.get("error").is_some_and(|error| !error.is_null()) {
        return Err("Messages response reported an upstream error".into());
    }
    validate_message_metadata(body)?;
    let blocks = body["content"].as_array().ok_or("Messages response has no content array")?;
    let stop = terminal_stop_reason(&body["stop_reason"])?;
    validate_stop_sequence(body)?;
    let usage = body.get("usage").cloned().unwrap_or_else(|| json!({}));
    validate_usage(&usage)?;
    validate_blocks(blocks, stop)?;
    Ok((blocks.clone(), stop.to_string(), usage))
}

pub(super) fn validate_message_metadata(body: &Value) -> Result<(), String> {
    if body.get("type").is_some_and(|kind| kind != "message")
        || body.get("role").is_some_and(|role| role != "assistant")
        || body.get("id").is_some_and(|id| !nonempty_text(id)) {
        return Err("Messages response has invalid message metadata".into());
    }
    Ok(())
}

pub(super) fn terminal_stop_reason(reason: &Value) -> Result<&str, String> {
    reason.as_str().filter(|reason| !reason.trim().is_empty())
        .ok_or_else(|| "Messages response has no terminal stop reason".into())
}

pub(super) fn validate_stop_sequence(body: &Value) -> Result<(), String> {
    if body.get("stop_sequence").is_some_and(|sequence| !sequence.is_null() && !sequence.is_string()) {
        return Err("Messages response has invalid stop sequence metadata".into());
    }
    Ok(())
}

pub(super) fn validate_usage(usage: &Value) -> Result<&Map<String, Value>, String> {
    let fields = usage.as_object().ok_or("Messages response has invalid usage metadata")?;
    if ["input_tokens", "output_tokens"].iter().any(|field|
        fields.get(*field).is_some_and(|count| count.as_u64().is_none())) {
        return Err("Messages response has invalid usage metadata".into());
    }
    Ok(fields)
}

pub(super) fn validate_blocks(blocks: &[Value], stop: &str) -> Result<(), String> {
    for block in blocks {
        if !block.is_object() || !nonempty_text(&block["type"]) {
            return Err("Messages response has an invalid content block".into());
        }
        match block["type"].as_str() {
            Some("text") if !block["text"].is_string() => {
                return Err("Messages response has invalid text content".into());
            }
            Some("tool_use" | "server_tool_use") => {
                if !nonempty_text(&block["id"]) || !nonempty_text(&block["name"]) {
                    return Err("Messages response has invalid tool identity".into());
                }
                // A token-limited call is retained only for the chain's existing
                // whole-chunk retry. It must never reach publication.
                if stop != "max_tokens" && !block["input"].is_object() {
                    return Err("Messages response has non-object tool input".into());
                }
            }
            _ => {}
        }
    }
    Ok(())
}
