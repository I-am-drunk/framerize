use serde_json::{json, Map, Value};

#[cfg(test)]
#[path = "messages_json_tests.rs"]
mod tests;

fn nonempty_text(value: &Value) -> bool {
    value.as_str().is_some_and(|text| !text.trim().is_empty())
}

pub(super) fn upstream_error(body: &Value) -> String {
    let error = body.get("error").filter(|value| value.is_object()).unwrap_or(body);
    let kind = error["type"].as_str().unwrap_or("api_error");
    let status = error.get("status").or_else(|| error.get("code"))
        .and_then(Value::as_u64).filter(|value| (400..600).contains(value))
        .unwrap_or(match kind {
            "invalid_request_error" => 400, "authentication_error" => 401,
            "permission_error" => 403, "not_found_error" => 404,
            "request_too_large" => 413, "rate_limit_error" => 429,
            "overloaded_error" => 529, _ => 500,
        });
    let message = error["message"].as_str()
        .unwrap_or("upstream error omitted its message");
    let mut fields = json!({"type":kind,
        "message":framer_core::error::safe_failure_summary(message)});
    if let Some(code) = error.get("code").filter(|value| value.is_string() || value.is_number()) {
        fields["code"] = code.clone();
    }
    format!("upstream {status} on /ai/v3/messages/: {}", json!({"error":fields}))
}

pub(super) fn decode(body: &Value) -> Result<(Vec<Value>, String, Value), String> {
    if !body.is_object() {
        return Err("Messages response is not an object".into());
    }
    if body["type"] == "error" || body.get("error").is_some_and(|error| !error.is_null()) {
        return Err(upstream_error(body));
    }
    validate_message_metadata(body)?;
    let blocks = body["content"].as_array().ok_or("Messages response has no content array")?;
    let stop = terminal_stop_reason(&body["stop_reason"])?;
    validate_stop_sequence(body)?;
    stop_details(body)?;
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

pub(super) fn stop_details(body: &Value) -> Result<Option<Value>, String> {
    match body.get("stop_details").filter(|details| !details.is_null()) {
        Some(details) if !details.is_object() =>
            Err("Messages response has invalid stop details metadata".into()),
        details => Ok(details.cloned()),
    }
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
