use serde_json::Value;

pub(super) struct OwnedAiPostResult {
    pub(super) status: i64,
    pub(super) usage: Option<String>,
    pub(super) text: String,
    pub(super) queue_wait_ms: u64,
}

/// Release the response envelope before the caller waits for its queue lock.
pub(super) fn take_ai_post_result(mut result: Value) -> OwnedAiPostResult {
    let status = result["status"].as_i64().unwrap_or(-1);
    let queue_wait_ms = result["queue_wait_ms"].as_u64().unwrap_or(0);
    let usage = take_string(&mut result, "usage");
    let text = take_string(&mut result, "body").unwrap_or_default();
    drop(result);
    OwnedAiPostResult { status, usage, text, queue_wait_ms }
}

fn take_string(result: &mut Value, key: &str) -> Option<String> {
    match result.get_mut(key).map(Value::take) {
        Some(Value::String(value)) => Some(value),
        _ => None,
    }
}
