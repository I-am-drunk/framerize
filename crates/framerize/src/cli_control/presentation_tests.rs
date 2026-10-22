use super::*;
use serde_json::json;

#[test]
fn nested_pool_error_reports_code_message_and_retry() {
    let reply = Reply { status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
        value: json!({"error": {"code": "framerize_pool_busy", "message": "Slot is busy"}}),
        retry_after: Some(1) };
    let error = check_status(&reply).unwrap_err();
    assert!(error.contains("framerize_pool_busy"));
    assert!(error.contains("Slot is busy"));
    assert!(error.contains("retry after 1 seconds"));
}

#[test]
fn recheck_delay_keeps_full_cooldown_visible() {
    let reply = Reply { status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
        value: json!({"error": {"code": "framerize_account_cooling",
            "retry_after_seconds": 30, "cooldown_remaining_seconds": 600}}),
        retry_after: None };
    let error = check_status(&reply).unwrap_err();
    assert!(error.contains("retry after 30 seconds"));
    assert!(error.contains("cooldown remaining 600 seconds"));
}

#[test]
fn legacy_top_level_error_remains_readable() {
    let reply = Reply { status: reqwest::StatusCode::BAD_REQUEST,
        value: json!({"code": "invalid_request", "message": "Invalid selection"}),
        retry_after: None };
    let error = check_status(&reply).unwrap_err();
    assert!(error.contains("invalid_request"));
    assert!(error.contains("Invalid selection"));
}
