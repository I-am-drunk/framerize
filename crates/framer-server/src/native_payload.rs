use super::*;

fn invalid() -> DispatchError { DispatchError::new(FailureKind::InvalidRequest) }

pub(super) fn messages(payload: Value) -> Result<Value, DispatchError> {
    if payload.get("system").is_some_and(|v| !v.is_string() && !v.is_array()) {
        return Err(invalid());
    }
    let mut prepared = convert::to_anthropic_request(ClientDialect::AnthropicMessages, &payload)
        .map_err(|_| invalid())?;
    // Preserve typed history, reasoning budgets and extension fields. Retain
    // only the existing explicit tool instruction addition from preparation.
    let Value::Object(fields) = payload else { return Err(invalid()); };
    prepared.as_object_mut().ok_or_else(invalid)?
        .extend(fields.into_iter().filter(|(key, _)| key != "system"));
    Ok(prepared)
}

pub(super) fn chat(payload: Value) -> Result<Value, DispatchError> {
    if payload.get("n").is_some_and(|n| n.as_u64() != Some(1)) { return Err(invalid()); }
    if payload.get("input").is_some() { return Err(invalid()); }
    let messages = payload["messages"].as_array().ok_or_else(invalid)?;
    for message in messages {
        let object = message.as_object().ok_or_else(invalid)?;
        if object.keys().any(|key| !matches!(key.as_str(),
            "role" | "content" | "tool_calls" | "tool_call_id")) { return Err(invalid()); }
        let role = message["role"].as_str().ok_or_else(invalid)?;
        if object.contains_key("tool_call_id") && role != "tool" { return Err(invalid()); }
        if object.contains_key("tool_calls")
            && (role != "assistant" || !message["tool_calls"].is_array()) {
            return Err(invalid());
        }
        if !matches!(role, "system" | "developer" | "user" | "assistant" | "tool") {
            return Err(invalid());
        }
        if let Some(parts) = message["content"].as_array() {
            for part in parts {
                let keys = part.as_object().ok_or_else(invalid)?;
                let allowed = match part["type"].as_str() {
                    Some("text") => keys.keys().all(|key| matches!(key.as_str(), "type" | "text")) && part["text"].is_string(),
                    Some("image_url") if role == "user" => valid_image(part),
                    _ => false,
                };
                if !allowed { return Err(invalid()); }
            }
        } else if !message["content"].is_string()
            && !(role == "assistant" && message["content"].is_null() && message["tool_calls"].is_array()) {
            return Err(invalid());
        }
    }
    let mut converted = convert::chat_to_responses_request(&payload).map_err(|_| invalid())?;
    let mut index = 0;
    for message in messages {
        let role = message["role"].as_str().ok_or_else(invalid)?;
        let calls = message["tool_calls"].as_array();
        if calls.is_some() && role != "assistant" { return Err(invalid()); }
        for call in calls.into_iter().flatten() {
            if call["type"].as_str() != Some("function") || !call["id"].is_string()
                || !call.as_object().is_some_and(|o| o.keys().all(|k| matches!(k.as_str(), "id" | "type" | "function")))
                || !call["function"].as_object().is_some_and(|o| o.keys().all(|k| k == "name" || k == "arguments"))
                || !call["function"]["name"].is_string() || !call["function"]["arguments"].is_string() {
                return Err(invalid());
            }
        }
        if role == "tool" {
            if !message["tool_call_id"].is_string() { return Err(invalid()); }
            if message["content"].is_array() {
                converted["input"][index]["output"] = parts(&message["content"], false);
            }
            index += 1;
        } else if role == "assistant" {
            let text = message["content"].as_str().map(str::to_owned).unwrap_or_else(|| {
                message["content"].as_array().into_iter().flatten()
                    .filter_map(|p| p["text"].as_str()).collect::<String>()
            });
            if !text.is_empty() {
                converted["input"][index]["content"] = parts(&message["content"], true);
                index += 1;
            } else if !message["content"].is_null() {
                converted["input"].as_array_mut().ok_or_else(invalid)?.insert(index,
                    serde_json::json!({"type":"message","role":"assistant",
                        "content":parts(&message["content"], true)}));
                index += 1;
            } else if calls.is_none_or(|c| c.is_empty()) { return Err(invalid()); }
            index += calls.map(Vec::len).unwrap_or(0);
        } else {
            converted["input"][index]["role"] = Value::String(role.to_owned());
            converted["input"][index]["content"] = parts(&message["content"], false);
            index += 1;
        }
    }
    preserve_fields(payload, converted)
}

fn valid_image(part: &Value) -> bool {
    if !part.as_object().is_some_and(|o| o.keys().all(|k| k == "type" || k == "image_url")) {
        return false;
    }
    let image = &part["image_url"];
    image.is_string() || image.as_object().is_some_and(|o| image["url"].is_string()
        && o.keys().all(|k| k == "url" || k == "detail")
        && image.get("detail").is_none_or(|detail| matches!(detail.as_str(),
            Some("auto") | Some("low") | Some("high"))))
}

fn parts(content: &Value, output: bool) -> Value {
    let text_type = if output { "output_text" } else { "input_text" };
    if let Some(text) = content.as_str() {
        return serde_json::json!([{ "type": text_type, "text": text }]);
    }
    Value::Array(content.as_array().into_iter().flatten().map(|part| {
        if part["type"] == "text" {
            serde_json::json!({"type":text_type,"text":part["text"]})
        } else {
            let image = &part["image_url"];
            let mut mapped = serde_json::json!({"type":"input_image",
                "image_url":image.as_str().or_else(|| image["url"].as_str()).unwrap_or_default()});
            if let Some(detail) = image.get("detail") { mapped["detail"] = detail.clone(); }
            mapped
        }
    }).collect())
}

fn preserve_fields(mut original: Value, mut mapped: Value) -> Result<Value, DispatchError> {
    if let Some(tools) = original.get("tools") {
        let mut converted = Vec::new();
        for tool in tools.as_array().ok_or_else(invalid)? {
            if tool["type"] != "function" || !tool["function"]["name"].is_string()
                || tool["function"].get("type").is_some()
                || !tool.as_object().is_some_and(|o| o.keys().all(|k| k == "type" || k == "function")) {
                return Err(invalid());
            }
            let mut function = tool["function"].clone();
            function["type"] = Value::String("function".into());
            converted.push(function);
        }
        mapped["tools"] = Value::Array(converted);
    }
    if let Some(choice) = original.get("tool_choice") {
        mapped["tool_choice"] = match choice {
            Value::String(value) if matches!(value.as_str(), "auto" | "none" | "required") => choice.clone(),
            Value::Object(object) if object.keys().all(|k| k == "type" || k == "function")
                && choice["type"] == "function"
                && choice["function"].as_object().is_some_and(|f| f.len() == 1
                    && f.get("name").is_some_and(Value::is_string)) =>
                serde_json::json!({"type":"function","name":choice["function"]["name"]}),
            _ => return Err(invalid()),
        };
    }
    if let Some(effort) = original.get("reasoning_effort").cloned() {
        if !effort.is_string() { return Err(invalid()); }
        if original.get("reasoning").is_some_and(|r| !r.is_object()
            || r.get("effort").is_some_and(|previous| previous != &effort)) { return Err(invalid()); }
        if original.get("reasoning").is_none() { original["reasoning"] = serde_json::json!({}); }
        original["reasoning"]["effort"] = effort;
    }
    if let Some(format) = original.get("response_format") {
        let object = format.as_object().ok_or_else(invalid)?;
        let format = match format["type"].as_str() {
            Some("json_object") | Some("text") if object.len() == 1 => format.clone(),
            Some("json_schema") if object.keys().all(|k| k == "type" || k == "json_schema")
                && format["json_schema"].is_object() && format["json_schema"].get("type").is_none() => {
                let mut value = format["json_schema"].clone();
                value["type"] = Value::String("json_schema".into());
                value
            }
            _ => return Err(invalid()),
        };
        let mut text = original.get("text").cloned().unwrap_or_else(|| serde_json::json!({}));
        if !text.is_object() || text.get("format").is_some_and(|old| old != &format) {
            return Err(invalid());
        }
        text["format"] = format;
        mapped["text"] = text;
    }
    let output = original.as_object_mut().ok_or_else(invalid)?;
    for key in ["messages", "tools", "tool_choice", "max_tokens", "max_completion_tokens",
        "n", "reasoning_effort", "response_format"] { output.remove(key); }
    let Value::Object(mapped) = mapped else { return Err(invalid()); };
    output.extend(mapped);
    Ok(original)
}

pub(super) fn responses(payload: Value) -> Result<Value, DispatchError> {
    // The shared chain builder drops these stateful Responses options.
    // Reject them explicitly until a native implementation supports them.
    if ["store", "background", "previous_response_id"].iter()
        .any(|key| payload.get(*key).is_some())
        || payload.get("instructions").is_some_and(|v| !v.is_string())
        || !(payload["input"].is_string() || payload["input"].is_array()) {
        return Err(invalid());
    }
    super::super::prepare_native_payload(ClientDialect::OpenAiResponses, payload)
        .map_err(|_| invalid())
}

#[cfg(test)]
#[path = "native_payload_ownership_tests.rs"]
mod ownership_tests;
