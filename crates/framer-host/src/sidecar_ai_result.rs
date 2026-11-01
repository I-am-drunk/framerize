use serde_json::Value;

pub(super) struct OwnedAiPostResult {
    pub(super) status: i64,
    pub(super) usage: Option<String>,
    pub(super) text: String,
    pub(super) queue_wait_ms: u64,
}

/// Release the response envelope before the caller waits for its queue lock.
pub(super) fn take_ai_post_result(mut result: Value) -> Result<OwnedAiPostResult, String> {
    let status = result["status"].as_i64()
        .ok_or("sidecar invalid AI reply: status is missing or not an integer")?;
    if !(-1..=0).contains(&status) && !(100..=599).contains(&status) {
        return Err("sidecar invalid AI reply: status is outside the HTTP range".into());
    }
    let queue_wait_ms = result["queue_wait_ms"].as_u64().unwrap_or(0);
    let usage = take_string(&mut result, "usage");
    let text = take_string(&mut result, "body")
        .ok_or("sidecar invalid AI reply: body is missing or not a string")?;
    drop(result);
    Ok(OwnedAiPostResult { status, usage, text, queue_wait_ms })
}

fn take_string(result: &mut Value, key: &str) -> Option<String> {
    match result.get_mut(key).map(Value::take) {
        Some(Value::String(value)) => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn invalid_host_envelopes_are_distinct_from_empty_upstream_bodies() {
        for (value, reason) in [
            (json!({"body":""}), "status is missing"),
            (json!({"status":600,"body":""}), "outside the HTTP range"),
            (json!({"status":200}), "body is missing"),
            (json!({"status":200,"body":{}}), "body is missing"),
        ] {
            let error = take_ai_post_result(value).err().expect("invalid reply");
            assert!(error.contains(reason), "{error}");
        }
        let result = take_ai_post_result(json!({"status":200,"body":""})).unwrap();
        assert_eq!(result.status, 200);
        assert_eq!(result.text, "");
    }
}
