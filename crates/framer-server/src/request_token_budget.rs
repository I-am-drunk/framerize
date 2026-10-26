use serde_json::Value;

pub(super) fn from_payload(payload: &Value) -> Result<u32, &'static str> {
    let limit = payload["max_tokens"].as_u64()
        .or_else(|| payload["max_completion_tokens"].as_u64())
        .or_else(|| payload["max_output_tokens"].as_u64())
        .unwrap_or(4096);
    u32::try_from(limit).map_err(|_| "output token limit exceeds 4294967295")
}

#[cfg(test)]
#[path = "request_token_budget_tests.rs"]
mod tests;
