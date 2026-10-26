use super::*;
use std::time::Duration;

fn config(metered_fallback: bool) -> ChainConfig {
    ChainConfig { chunk_tokens: 600, max_chunks: 1, max_total_tokens: Some(4096),
        metered_fallback, ..ChainConfig::default() }
}

fn refused(status: u16, body: &str) -> Reply {
    Reply { streaming: false, status, result: Ok(body.into()) }
}

#[test]
fn second_attempt_refusals_do_not_start_metered_fallback() {
    for enabled in [true, false] {
        for (status, body, expected) in [
            (402, "no credits", "credits-exhausted"),
            (403, "Forbidden", "risk-refused"),
            (403, "provider policy", "upstream 403 on /ai/v3/messages/: provider policy"),
            (400, "bad request", "retryable 400 on /ai/v3/messages/: bad request"),
            (429, "busy", "retryable 429 on /ai/v3/messages/: busy"),
        ] {
            let observed = run_with_config(vec![failed_request(), refused(status, body)], config(enabled));
            assert_eq!(observed.result.unwrap_err(), expected);
            assert!(observed.emitted.is_empty());
            assert_eq!(observed.requests.len(), 2);
            assert_eq!(observed.refunds, 0);
        }
    }
}

#[test]
fn retry_eligibility_retains_auth_and_idle_boundaries() {
    for elapsed in [0.0, 8.0, 9.0] {
        for error in ["unauthorized", "credits-exhausted", "upstream 403: policy",
            "retryable 400: bad request", "retryable 429: busy"] {
            assert!(!retryable_failure(error, elapsed), "{error} at {elapsed}");
        }
        assert!(retryable_failure("fixture transport closed", elapsed));
        assert!(retryable_failure("retryable 502: bad gateway", elapsed));
        assert_eq!(retryable_failure("risk-refused", elapsed), elapsed > 8.0);
    }
}

#[test]
fn delayed_second_attempt_keeps_idle_fallback() {
    let observed = run_with_delays(vec![failed_request(), refused(403, "Forbidden"),
        streamed(stream(tool(), "tool_use"))], config(true),
        vec![Duration::ZERO, Duration::from_secs(9)]);
    let response = observed.result.unwrap();
    assert_eq!(response.content, vec![tool()]);
    assert_eq!(response.stop_reason, "tool_use");
    assert_eq!(response.metered_chunks, 1);
    assert_eq!(observed.requests.len(), 3);
    assert_eq!(observed.emitted, vec![tool()]);
    assert_eq!(observed.refunds, 1);
}

#[test]
fn prior_attempt_time_cannot_make_a_fast_refusal_retryable() {
    let observed = run_with_delays(vec![failed_request(), refused(403, "Forbidden")],
        config(true), vec![Duration::from_secs(9), Duration::ZERO]);
    assert_eq!(observed.result.unwrap_err(), "risk-refused");
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn local_cooldown_cannot_make_a_fast_refusal_retryable() {
    let prefix = json!({"type":"text", "text":"prefix"});
    let observed = run_with_delays(vec![buffered(prefix.clone(), "max_tokens"),
        failed_request(), refused(0, "velocity-paused:fixture; retry in 19s"),
        refused(403, "Forbidden")], ChainConfig { max_chunks: 2, ..config(true) },
        vec![Duration::ZERO, Duration::ZERO, Duration::from_secs(9), Duration::ZERO]);
    assert_eq!(observed.result.unwrap_err(), "risk-refused");
    assert_eq!(observed.emitted, vec![prefix]);
    assert_eq!(observed.requests.len(), 4);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn disabled_metered_fallback_keeps_transport_error() {
    let observed = run_with_config(vec![failed_request(), failed_request()], config(false));
    assert_eq!(observed.result.unwrap_err(),
        "chunk 0 retry died (fallback off): fixture transport closed");
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.refunds, 0);
}
