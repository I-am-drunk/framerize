use super::*;
use std::time::Duration;

#[test]
fn successful_inflight_reply_is_kept_after_the_active_deadline() {
    let reply = buffered(json!({"type":"text","text":"complete"}), "end_turn");
    let observed = run_with_delays(vec![reply], ChainConfig::default(),
        vec![Duration::from_secs(301)]);
    assert_eq!(observed.result.unwrap().stop_reason, "end_turn");
    assert_eq!(observed.requests.len(), 1);
    assert_eq!(observed.emitted.len(), 1);
}

#[test]
fn rate_backoff_cannot_dispatch_after_active_deadline() {
    let reply = Reply { streaming: false, status: 429,
        result: Ok("provider concurrency limit exceeded".into()) };
    let observed = run_with_delays(vec![reply], ChainConfig::default(),
        vec![Duration::from_secs(299)]);
    let error = observed.result.unwrap_err();
    assert!(error.contains("300s active-time budget"), "{error}");
    assert!(error.contains("429"), "{error}");
    assert_eq!(observed.requests.len(), 1);
    assert!(observed.emitted.is_empty());
}

#[test]
fn acknowledged_tool_retry_cannot_dispatch_after_active_deadline() {
    let observed = run_with_delays(vec![buffered(tool(), "max_tokens")],
        ChainConfig { metered_fallback: true, ..ChainConfig::default() },
        vec![Duration::from_secs(301)]);
    let error = observed.result.unwrap_err();
    assert!(error.contains("300s active-time budget"), "{error}");
    assert_eq!(observed.requests.len(), 1);
    assert!(observed.emitted.is_empty());
}

#[test]
fn transport_failure_keeps_its_cause_after_active_deadline() {
    let observed = run_with_delays(vec![failed_request()],
        ChainConfig { metered_fallback: false, ..ChainConfig::default() },
        vec![Duration::from_secs(301)]);
    let error = observed.result.unwrap_err();
    assert!(error.contains("fixture transport closed"), "{error}");
    assert_eq!(observed.requests.len(), 1);
    assert!(observed.emitted.is_empty());
}
