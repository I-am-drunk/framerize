use serde_json::Value;

pub(super) fn from_payload(payload: &Value) -> Result<u32, &'static str> {
    explicit_from_payload(payload).map(|limit| limit.unwrap_or(4096))
}

pub(super) fn explicit_from_payload(payload: &Value) -> Result<Option<u32>, &'static str> {
    let limit = payload["max_tokens"].as_u64()
        .or_else(|| payload["max_completion_tokens"].as_u64())
        .or_else(|| payload["max_output_tokens"].as_u64());
    limit.map(u32::try_from).transpose()
        .map_err(|_| "output token limit exceeds 4294967295")
}

#[cfg(test)]
#[path = "request_token_budget_tests.rs"]
mod tests;
