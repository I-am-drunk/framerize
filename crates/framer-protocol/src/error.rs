use thiserror::Error;

pub const ADAPTIVE_INCOMPLETE_PREFIX: &str = "adaptive-buffered-transport: ";
pub const ADAPTIVE_INCOMPLETE_CODE: &str = "framerize_adaptive_response_incomplete";

pub fn safe_failure_summary(error: &str) -> String {
    let mut line = error.lines().next().unwrap_or("").to_owned();
    let lower = line.to_ascii_lowercase();
    if let Some(index) = ["bearer ", "cookie=", "cookie:", "password=", "password:",
        "authorization:", "token=", "token:", "\"token\"", "session=", "session:",
        "\"authorization\"", "\"password\"", "\"cookie\"", "\"session\"",
        "accesstoken", "access_token", "cookie_header"]
        .iter().filter_map(|marker| lower.find(marker)).min() {
        line.truncate(index);
        line.push_str("<redacted>");
    }
    line.split_whitespace().map(|word| {
        if word.contains("http://") || word.contains("https://") { "<url>" }
        else if word.len() >= 64 && word.bytes().all(|b|
            b.is_ascii_alphanumeric() || b"._+/=-".contains(&b)) { "<redacted>" }
        else { word }
    }).collect::<Vec<_>>().join(" ").chars().take(600).collect()
}

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

pub fn upstream_status_body(error: &str) -> Option<(u16, &str)> {
    let cause = chain_error_cause(error);
    let rest = cause.strip_prefix("retryable ")
        .or_else(|| cause.strip_prefix("upstream "))?;
    let (status, rest) = rest.split_once(" on ")?;
    let status = status.parse::<u16>().ok().filter(|n| (400..600).contains(n))?;
    let (path, body) = rest.split_once(": ")?;
    if !path.starts_with('/') || path.chars().any(char::is_whitespace) {
        return None;
    }
    Some((status, body))
}

pub fn is_workspace_saturation(error: &str) -> bool {
    let Some((429, body)) = upstream_status_body(error) else { return false; };
    let parsed = serde_json::from_str::<serde_json::Value>(body).ok();
    let message = parsed.as_ref().and_then(|value|
        value.pointer("/error/message").or_else(|| value.get("message")))
        .and_then(serde_json::Value::as_str).unwrap_or(body);
    message.trim().eq_ignore_ascii_case("too many concurrent AI requests for this workspace")
}

/// A smaller model chunk cannot repair a local resource or ownership failure.
pub fn is_local_request_failure(error: &str) -> bool {
    let cause = chain_error_cause(error);
    matches!(cause, "castle-unavailable" | "castle-v3-unavailable"
        | "browser cleanup unconfirmed" | "device request budget unavailable")
        || ["Low available RAM:", "browser allocation deferred", "sidecar ",
            "host-lifecycle-unconfirmed:", "account rate ",
            "incompatible account rate history", "browser preparation failed",
            "account browser cleanup", "browser cleanup unconfirmed",
            "browser token ", "host task ownership", "host startup:", "host admission:"]
            .iter().any(|prefix| cause.starts_with(prefix))
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
        let remaining = cause.strip_prefix("account-cooling:")?;
        let seconds = remaining.split_once("; ").map_or(remaining, |(value, _)| value);
        let retry_after_secs = seconds.parse::<u64>().ok()?;
        Some(Self { retry_after_secs: retry_after_secs.max(1) })
    }
}

/// Presentation only. The original error remains the health/cleanup evidence.
pub(crate) fn public_unavailable_message(error: &str) -> Option<String> {
    let cause = chain_error_cause(error);
    let local = cause.strip_prefix("retryable 0 on ")
        .and_then(|rest| rest.split_once(": "))
        .filter(|(path, _)| path.starts_with("/ai/") && !path.chars().any(char::is_whitespace))
        .map_or(cause, |(_, detail)| detail);
    if let Some(detail) = local.strip_prefix("host-lifecycle-unconfirmed: ") {
        return Some(format!("Browser host failure: {}. Browser cleanup remains unconfirmed.",
            safe_failure_summary(detail)));
    }
    if let Some(detail) = local.strip_prefix("velocity-paused: ") {
        return Some(format!("The selected browser device cannot send this request: {}",
            safe_failure_summary(detail)));
    }
    if ["sidecar timeout after ", "sidecar account command timed out",
        "sidecar write:", "sidecar invalid AI reply:", "browser preparation failed",
        "host startup:", "host admission:"]
        .iter().any(|prefix| local.starts_with(prefix)) {
        return Some(format!("Browser host failure: {}", safe_failure_summary(local)));
    }
    if ["Low available RAM:", "browser token ", "browser cleanup unconfirmed:"]
        .iter().any(|prefix| cause.starts_with(prefix)) {
        return Some(safe_failure_summary(cause));
    }
    if let Some(cooldown) = AccountCooldown::from_error(error) {
        let reason = cause.split_once("; ").map(|(_, reason)| safe_failure_summary(reason))
            .unwrap_or_else(|| "the saved account cooldown has no recorded triggering cause".into());
        return Some(format!("Cannot start the request because {reason}. Retry in {} seconds.",
                           cooldown.retry_after_secs));
    }
    let server_wait = cause.strip_prefix("server-cooling:")
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|value| value.parse::<u64>().ok());
    if let Some(seconds) = server_wait {
        let seconds = seconds.max(1);
        let unit = if seconds == 1 { "second" } else { "seconds" };
        return Some(format!("Recent security refusals across multiple accounts triggered a global pause. Retry in {seconds} {unit}."));
    }
    if is_security_refusal(error) {
        return Some("Framer refused the request with HTTP 403 during its security checks. Its response did not disclose a more specific cause.".into());
    }
    local_unavailable_message(cause).map(str::to_owned)
}

fn local_unavailable_message(error: &str) -> Option<&'static str> {
    let cause = error.strip_prefix("retryable 0 on ")
        .and_then(|rest| rest.split_once(": "))
        .filter(|(path, _)| path.starts_with("/ai/") && !path.chars().any(char::is_whitespace))
        .map_or(error, |(_, cause)| cause);
    if cause.starts_with("sidecar timeout after ")
        || cause.starts_with("sidecar account command timed out") {
        return Some("The browser host did not complete its command before the deadline. Pending browser work still requires confirmed cleanup.");
    }
    if cause.starts_with("host-lifecycle-unconfirmed: sidecar exited") {
        return Some("The browser host process exited before completing the request. Its browser cleanup has not been confirmed.");
    }
    if cause.starts_with("host-lifecycle-unconfirmed: sidecar output closed")
        || cause == "sidecar dropped the request" {
        return Some("The browser host's reply channel closed before a result arrived. Its browser cleanup has not been confirmed.");
    }
    if cause.starts_with("sidecar write:") {
        return Some("Writing the request to the browser host's command pipe failed. The request's cleanup has not been confirmed.");
    }
    if matches!(cause, "castle-unavailable" | "castle-v3-unavailable") {
        return Some("The browser's Castle SDK did not supply a usable request token. The host did not report a more specific SDK failure.");
    }
    if cause == "device request budget unavailable" || cause.starts_with("velocity-paused:") {
        return Some("The selected browser device has exhausted its local request allowance or is still under a security cooldown.");
    }
    if matches!(cause, "account browser cleanup is unconfirmed; retry after cleanup"
        | "browser launch cleanup is unconfirmed" | "browser cleanup unconfirmed"
        | "account browser cleanup is unconfirmed or closing"
        | "host task ownership unconfirmed" | "sidecar dropped the request")
        || ["host-lifecycle-unconfirmed:", "sidecar timeout after ",
            "sidecar account command timed out", "sidecar write:", "account residency fence failed:"]
            .iter().any(|prefix| cause.starts_with(prefix))
    {
        return Some("Earlier browser work ended without a confirmed cleanup result. Its original failure was not recorded; the retained resources cannot be reused safely.");
    }
    None
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "error_cause_tests.rs"]
mod cause_tests;
