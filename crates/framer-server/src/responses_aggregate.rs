use serde_json::Value;

pub(super) fn parse(body: &str) -> Result<Value, &'static str> {
    let mut terminal = None;
    let mut data = Vec::new();
    let mut event_name = "";
    // SSE permits CR alone; a CRLF pair must still end just one line.
    let lines = body.split_terminator('\n').flat_map(|line| {
        line.strip_suffix('\r').unwrap_or(line).split('\r')
    });
    for line in lines.chain(std::iter::once("")) {
        if let Some(value) = line.strip_prefix("event:") {
            event_name = value.strip_prefix(' ').unwrap_or(value);
        } else if line == "event" {
            event_name = "";
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.strip_prefix(' ').unwrap_or(value));
        } else if line.is_empty() {
            consume(&data.join("\n"), std::mem::take(&mut event_name), &mut terminal)?;
            data.clear();
        }
    }
    terminal.ok_or("Responses stream has no terminal event")
}

fn consume(data: &str, name: &str, terminal: &mut Option<Value>) -> Result<(), &'static str> {
    match name {
        "error" => return Err("Responses stream ended with error"),
        "response.failed" => return Err("Responses stream ended with response.failed"),
        _ => {}
    }
    let named_terminal = matches!(name, "response.completed" | "response.incomplete");
    if !named_terminal && (data == "[DONE]" || data.is_empty()) { return Ok(()); }
    let parsed = serde_json::from_str::<Value>(data);
    if named_terminal && parsed.as_ref().ok()
        .and_then(|event| event.get("type")).and_then(Value::as_str) != Some(name) {
        return Err("Responses stream has an invalid named terminal event");
    }
    let event = parsed
        .map_err(|_| "Responses stream has malformed event JSON")?;
    let kind = event.get("type").and_then(Value::as_str)
        .ok_or("Responses stream has an invalid event envelope")?;
    match kind {
        "error" => return Err("Responses stream ended with error"),
        "response.failed" => return Err("Responses stream ended with response.failed"),
        "response.completed" | "response.incomplete" => {}
        _ => return Ok(()),
    }
    let response = event.get("response").filter(|v| v.is_object())
        .ok_or("Responses stream has an invalid terminal response")?;
    if !response.get("output").is_some_and(Value::is_array) {
        return Err("Responses stream has an invalid output array");
    }
    if response.get("status").is_some_and(|value| {
        !matches!(value.as_str(), Some("completed" | "incomplete"))
    }) || (kind == "response.incomplete" && response["status"] != "incomplete") {
        return Err("Responses stream has a contradictory terminal status");
    }
    if response.get("usage").is_some_and(|value| !value.is_null() && !value.is_object()) {
        return Err("Responses stream has an invalid usage object");
    }
    *terminal = Some(response.clone());
    Ok(())
}

#[cfg(test)]
#[path = "responses_aggregate_tests.rs"]
mod tests;
