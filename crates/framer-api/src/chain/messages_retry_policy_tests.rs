use super::*;
use std::time::Duration;

fn config(metered_fallback: bool) -> ChainConfig {
    ChainConfig { chunk_tokens: 600, max_chunks: 1, max_total_tokens: Some(4096),
        metered_fallback, ..ChainConfig::default() }
}

#[test]
fn two_slow_valid_replies_keep_their_output() {
    let first = json!({"type":"text", "text":"first"});
    let last = json!({"type":"text", "text":"last"});
    let observed = run_with_delays(vec![buffered(first.clone(), "max_tokens"),
        buffered(last.clone(), "end_turn")], ChainConfig { max_chunks:2, ..config(false) },
        vec![Duration::from_secs(20), Duration::from_secs(20)]);
    assert_eq!(observed.result.unwrap().stop_reason, "end_turn");
    assert_eq!(observed.emitted, vec![first, last]);
    assert_eq!(observed.requests.len(), 2);
}

fn refused(status: u16, body: &str) -> Reply {
    Reply { streaming: false, status, result: Ok(body.into()) }
}

#[test]
fn exhausted_provider_rate_limit_keeps_its_cause_and_request_route() {
    let replies = (0..10).map(|_| refused(429, "upstream_provider_shared_pool")).collect();
    let observed = run_with_config(replies, config(true));
    assert_eq!(observed.result.unwrap_err(),
        "retryable 429 on /ai/v3/messages/: upstream_provider_shared_pool");
    assert_eq!(observed.requests.len(), 10);
    for request in &observed.requests[1..] {
        for field in ["featureCategory", "featureName", "step", "request"] {
            assert_eq!(request[field], observed.requests[0][field], "{field}");
        }
    }
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.refunds, 0);
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
        assert!(!retryable_failure("risk-refused", elapsed));
    }
}

#[test]
fn delayed_security_refusal_does_not_start_fallback() {
    let observed = run_with_delays(vec![failed_request(), refused(403, "Forbidden")], config(true),
        vec![Duration::ZERO, Duration::from_secs(9)]);
    assert_eq!(observed.result.unwrap_err(), "risk-refused");
    assert_eq!(observed.requests.len(), 2);
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.refunds, 0);
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
