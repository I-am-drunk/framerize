use super::http::Reply;
use serde_json::Value;
use std::io::Write;

fn text(value: &str) -> String {
    value.chars().filter(|character| !character.is_control()).take(512).collect()
}

fn write_output(value: &str) -> Result<(), String> {
    let output = std::io::stdout();
    let mut output = output.lock();
    output.write_all(value.as_bytes()).and_then(|_| output.write_all(b"\n"))
        .and_then(|_| output.flush()).map_err(|_| "could not write standard output".into())
}

fn pretty(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|_| "could not format the control response".into())
}

fn check_status(reply: &Reply) -> Result<(), String> {
    if reply.status.is_success() { return Ok(()); }
    let details = reply.value.get("error").filter(|value| value.is_object())
        .unwrap_or(&reply.value);
    let mut error = format!("daemon returned HTTP {}", reply.status.as_u16());
    if let Some(code) = details.get("code").and_then(Value::as_str) {
        error.push_str(&format!(" ({})", text(code)));
    }
    if let Some(message) = details.get("message").and_then(Value::as_str) {
        error.push_str(&format!(": {}", text(message)));
    }
    if let Some(seconds) = reply.retry_after.or_else(|| details.get("retry_after_seconds")
        .or_else(|| details.get("retry_after_secs")).and_then(Value::as_u64)) {
        error.push_str(&format!("; retry after {seconds} seconds"));
    }
    if let Some(seconds) = details.get("cooldown_remaining_seconds").and_then(Value::as_u64) {
        error.push_str(&format!("; cooldown remaining {seconds} seconds"));
    }
    match reply.status.as_u16() {
        404 => error.push_str("; this daemon may not provide that control endpoint"),
        409 => error.push_str("; inspect pool status for occupied or busy slots"),
        _ => {}
    }
    Err(error)
}

fn accepted_notice(reply: &Reply) {
    if reply.status == reqwest::StatusCode::ACCEPTED {
        eprintln!("Operation accepted; inspect pool status for completion.");
    }
}

fn redact_account_proxies(value: &mut Value) {
    if let Some(accounts) = value.as_array_mut() {
        for account in accounts {
            if let Some(fields) = account.as_object_mut() {
                let configured = fields.remove("proxy").is_some_and(|proxy| !proxy.is_null());
                fields.insert("proxy_configured".into(), Value::Bool(configured));
            }
        }
    }
}

fn account_lines(value: &Value) -> Result<String, String> {
    let accounts = value.as_array().ok_or("account list response must be an array")?;
    if accounts.is_empty() { return Ok("No saved accounts. Use framerize login to add one.".into()); }
    let mut lines = vec!["ACCOUNT ID                            STATE           NAME".to_string()];
    for account in accounts {
        let id = text(account.get("id").and_then(Value::as_str).ok_or("account response lacks id")?);
        let state = text(account.get("state").and_then(Value::as_str).unwrap_or("unknown"));
        let name = text(account.get("email").and_then(Value::as_str)
            .or_else(|| account.get("alias").and_then(Value::as_str)).unwrap_or(""));
        lines.push(format!("{id:36}  {state:14}  {name}"));
    }
    Ok(lines.join("\n"))
}

pub(super) fn finish_accounts(mut reply: Reply, json: bool, listing: bool) -> Result<(), String> {
    if listing && reply.status.is_success() { redact_account_proxies(&mut reply.value); }
    if json {
        write_output(&pretty(&reply.value)?)?;
    } else if reply.status.is_success() {
        let output = if listing { account_lines(&reply.value)? } else { pretty(&reply.value)? };
        write_output(&output)?;
    }
    accepted_notice(&reply);
    check_status(&reply)
}

fn pool_lines(value: &Value) -> Result<String, String> {
    let config = value.get("config").unwrap_or(value);
    let Some(capacity) = config.get("active_capacity").and_then(Value::as_u64) else {
        return pretty(value);
    };
    let eligible = match config.get("eligible_accounts") {
        Some(Value::String(mode)) if mode == "all" => "all saved accounts".to_string(),
        Some(selection) => serde_json::to_string(selection).map_err(|_| "invalid eligibility response")?,
        None => "not reported".into(),
    };
    let mut output = format!("Active browser capacity: {capacity}\nEligible accounts: {eligible}");
    let mut remaining = value.clone();
    if let Some(fields) = remaining.as_object_mut() {
        if fields.remove("config").is_none() {
            fields.remove("active_capacity");
            fields.remove("eligible_accounts");
        }
        if !fields.is_empty() { output.push_str(&format!("\n{}", pretty(&remaining)?)); }
    }
    Ok(output)
}

pub(super) fn finish_pool(reply: Reply, json: bool) -> Result<(), String> {
    if json { write_output(&pretty(&reply.value)?)?; }
    else if reply.status.is_success() { write_output(&pool_lines(&reply.value)?)?; }
    accepted_notice(&reply);
    check_status(&reply)
}

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod tests;
