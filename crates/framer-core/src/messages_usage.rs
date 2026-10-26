use super::AssembledResponse;
use serde_json::{json, Value};

pub(super) fn snapshot(response: &AssembledResponse) -> Value {
    let mut usage = if response.usage.is_object() { response.usage.clone() } else { json!({}) };
    usage["input_tokens"] = json!(response.input_tokens);
    usage["output_tokens"] = json!(0);
    usage
}

pub(super) fn delta(response: &AssembledResponse) -> Value {
    let mut usage = json!({"input_tokens":response.input_tokens,
        "output_tokens":response.output_tokens});
    // These cumulative fields can update an already-open Messages snapshot.
    for field in ["cache_creation_input_tokens", "cache_read_input_tokens",
        "server_tool_use", "output_tokens_details"] {
        if let Some(value) = response.usage.get(field) { usage[field] = value.clone(); }
    }
    usage
}
