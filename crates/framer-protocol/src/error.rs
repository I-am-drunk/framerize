use thiserror::Error;

/// Remove only the chain wrapper this protocol produces, never upstream text.
pub fn chain_error_cause(error: &str) -> &str {
    error.strip_prefix("chunk ")
        .and_then(|rest| rest.split_once(" retry died (fallback off): "))
        .filter(|(index, _)| !index.is_empty()
            && index.bytes().all(|byte| byte.is_ascii_digit())
            && index.parse::<u32>().is_ok())
        .map_or(error, |(_, cause)| cause)
}

/// Security accounting accepts the exact internal sentinel only.
pub fn is_security_refusal(error: &str) -> bool {
    chain_error_cause(error) == "risk-refused"
}

/// Local account capacity, distinct from an upstream security refusal.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[error("account-cooling:{retry_after_secs}")]
pub struct AccountCooldown {
    pub retry_after_secs: u64,
}

impl AccountCooldown {
    pub fn from_error(error: &str) -> Option<Self> {
        let cause = chain_error_cause(error);
        let retry_after_secs = cause.strip_prefix("account-cooling:")?.parse::<u64>().ok()?;
        Some(Self { retry_after_secs: retry_after_secs.max(1) })
    }
}

/// Presentation only. The original error remains the health/cleanup evidence.
pub(crate) fn public_unavailable_message(error: &str) -> Option<String> {
    let cause = chain_error_cause(error);
    if let Some(cooldown) = AccountCooldown::from_error(error) {
        return Some(format!("This account is temporarily cooling down. Retry in {} seconds.",
            cooldown.retry_after_secs));
    }
    let server_wait = cause.strip_prefix("server-cooling:")
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|value| value.parse::<u64>().ok());
    if let Some(seconds) = server_wait {
        let seconds = seconds.max(1);
        let unit = if seconds == 1 { "second" } else { "seconds" };
        return Some(format!("Framerize is temporarily unavailable. Retry in {seconds} {unit}."));
    }
    if is_security_refusal(error) {
        return Some("The upstream service is temporarily unavailable. Please retry shortly.".into());
    }
    local_unavailable_message(cause).map(str::to_owned)
}

fn local_unavailable_message(error: &str) -> Option<&'static str> {
    let cause = error.strip_prefix("retryable 0 on ")
        .and_then(|rest| rest.split_once(": "))
        .filter(|(path, _)| path.starts_with("/ai/") && !path.chars().any(char::is_whitespace))
        .map_or(error, |(_, cause)| cause);
    if cause == "castle-unavailable" {
        return Some("Framerize could not prepare the request. Please retry shortly.");
    }
    if cause == "device request budget unavailable" || cause.starts_with("velocity-paused:") {
        return Some("No account is ready for this request yet. Please retry shortly.");
    }
    if matches!(cause, "account browser cleanup is unconfirmed; retry after cleanup"
        | "browser launch cleanup is unconfirmed" | "browser cleanup unconfirmed"
        | "account browser cleanup is unconfirmed or closing"
        | "host task ownership unconfirmed" | "sidecar dropped the request")
        || ["host-lifecycle-unconfirmed:", "sidecar timeout after ",
            "sidecar account command timed out", "sidecar write:", "account residency fence failed:"]
            .iter().any(|prefix| cause.starts_with(prefix))
    {
        return Some("Framerize cannot safely start another request yet. Please retry later.");
    }
    None
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
