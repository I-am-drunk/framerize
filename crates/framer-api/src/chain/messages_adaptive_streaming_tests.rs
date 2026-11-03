use super::*;
use framer_core::convert;

fn adaptive_config() -> ChainConfig {
    ChainConfig { max_total_tokens: Some(64_000), metered_fallback: true,
        ..ChainConfig::default() }
}

fn adaptive_payload() -> Value {
    json!({"model":"claude-fable-5-1", "thinking":{"type":"adaptive"},
        "output_config":{"effort":"max"}, "speed":"fast",
        "messages":[{"role":"user","content":"fixture"}]})
}

#[test]
fn opted_in_adaptive_stream_keeps_completed_refusal_without_replay() {
    let details = json!({"type":"refusal","category":"reasoning_extraction",
        "explanation":"Provider explanation"});
    let raw = [
        json!({"type":"message_start","message":{"id":"fixture-refusal"}}),
        json!({"type":"message_delta","delta":{"stop_reason":"refusal",
            "stop_details":details},"usage":{"output_tokens":1}}),
        json!({"type":"message_stop"}),
    ].iter().map(|v| format!("data: {v}\n\n")).collect();
    let observed = run_with_payload(vec![streamed(raw)], adaptive_config(),
        Vec::new(), adaptive_payload());
    let response = observed.result.unwrap();
    assert_eq!(response.stop_reason, "refusal");
    assert_eq!(response.stop_details, Some(details));
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.requests.len(), 1);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn opted_in_adaptive_generation_streams_before_any_buffered_attempt() {
    let observed = run_with_payload(vec![streamed(stream(tool(), "tool_use"))],
        adaptive_config(), Vec::new(), adaptive_payload());
    let response = observed.result.unwrap();
    assert_eq!(response.content, vec![tool()]);
    assert_eq!(response.metered_chunks, 1);
    assert_eq!(observed.requests.len(), 1);
    let sent = &observed.requests[0]["request"];
    assert_eq!(sent["stream"], true);
    assert_eq!(sent["max_tokens"], 64_000);
    assert_eq!(sent["thinking"], adaptive_payload()["thinking"]);
    assert_eq!(sent["output_config"], adaptive_payload()["output_config"]);
    assert_eq!(sent["messages"], adaptive_payload()["messages"]);
    assert_eq!(sent["speed"], "fast");
    assert_eq!(observed.refunds, 0);
}

#[test]
fn long_adaptive_stream_keeps_its_result_without_replay() {
    let observed = run_with_payload(vec![streamed(stream(tool(), "tool_use"))],
        adaptive_config(), vec![std::time::Duration::from_secs(90)], adaptive_payload());
    let result = observed.result.unwrap();
    assert_eq!(result.content, vec![tool()]);
    assert_eq!(result.output_tokens, 80);
    assert_eq!(observed.emitted, vec![tool()]);
    assert_eq!(observed.requests.len(), 1);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn adaptive_stream_refusals_never_retry_or_request_refunds() {
    for status in [403, 429, 503] {
        let reply = Reply { streaming: true, status,
            result: Ok("fixture upstream refusal".into()) };
        let observed = run_with_payload(vec![reply], adaptive_config(),
            Vec::new(), adaptive_payload());
        assert!(observed.result.is_err());
        assert!(observed.emitted.is_empty());
        assert_eq!(observed.requests.len(), 1);
        assert_eq!(observed.refunds, 0);
    }
}

#[test]
fn adaptive_truncated_stream_never_publishes_a_tool() {
    let text = stream(tool(), "max_tokens");
    let observed = run_with_payload(vec![streamed(text)], adaptive_config(),
        Vec::new(), adaptive_payload());
    assert!(observed.result.unwrap_err().contains("incomplete tool call"));
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.requests.len(), 1);
}

#[test]
fn adaptive_transport_loss_does_not_replay_with_a_smaller_allowance() {
    let reply = Reply { streaming: true, status: 200,
        result: Err("fixture transport closed".into()) };
    let observed = run_with_payload(vec![reply], adaptive_config(),
        Vec::new(), adaptive_payload());
    let error = observed.result.unwrap_err();
    assert!(error.starts_with(framer_core::error::ADAPTIVE_INCOMPLETE_PREFIX));
    assert!(error.contains("fixture transport closed"));
    let (_, body) = framer_core::convert::error_json(
        framer_core::convert::ClientDialect::AnthropicMessages, &error);
    assert_eq!(body["error"]["retryable"], false);
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.requests.len(), 1);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn free_adaptive_transport_failure_explains_the_allowance_and_cause() {
    let mut config = adaptive_config();
    config.metered_fallback = false;
    let reply = Reply { streaming: false, status: 200,
        result: Err("in-page fetch failed: TypeError: NetworkError".into()) };
    let observed = run_with_payload(vec![reply], config,
        vec![std::time::Duration::from_secs(61)], adaptive_payload());
    let error = observed.result.unwrap_err();
    assert!(error.starts_with(framer_core::error::ADAPTIVE_INCOMPLETE_PREFIX));
    assert!(error.contains("600 output tokens"));
    assert!(error.contains("61.0s"));
    assert!(error.contains("NetworkError"));
    assert!(error.contains("completion is unknown"));
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.requests.len(), 1);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn hidden_gateway_timeout_keeps_observed_status_and_prevents_replay() {
    let mut config = adaptive_config();
    config.metered_fallback = false;
    let reply = Reply { streaming: false, status: 200,
        result: Err("in-page fetch failed: browser observed upstream HTTP 504 Gateway Timeout after 60.1s. Browser error: TypeError: NetworkError".into()) };
    let observed = run_with_payload(vec![reply], config,
        vec![std::time::Duration::from_secs(61)], adaptive_payload());
    let error = observed.result.unwrap_err();
    assert!(error.starts_with(framer_core::error::ADAPTIVE_INCOMPLETE_PREFIX));
    assert!(error.contains("HTTP 504 Gateway Timeout"));
    assert!(error.contains("completion is unknown"));
    assert_eq!(observed.requests.len(), 1);
    assert_eq!(observed.refunds, 0);
    assert!(observed.emitted.is_empty());
}

#[test]
fn buffered_gateway_failures_preserve_cause_and_disable_replay() {
    for status in [504, 502] {
        let mut config = adaptive_config();
        config.metered_fallback = false;
        let reply = Reply { streaming: false, status,
            result: Ok("upstream execution status unknown".into()) };
        let observed = run_with_payload(vec![reply], config,
            vec![std::time::Duration::from_secs(61)], adaptive_payload());
        let error = observed.result.unwrap_err();
        assert!(error.starts_with(framer_core::error::ADAPTIVE_INCOMPLETE_PREFIX), "{error}");
        assert!(error.contains(&format!("{status} on /ai/v3/messages/")));
        assert!(error.contains("upstream execution status unknown"));
        assert!(error.contains("completion is unknown"));
        assert!(error.contains("61.0s"));
        let (_, body) = convert::error_json(convert::ClientDialect::AnthropicMessages, &error);
        assert_eq!(body["error"]["retryable"], false);
        assert_eq!(body["error"]["code"], framer_core::error::ADAPTIVE_INCOMPLETE_CODE);
        assert_eq!(observed.requests.len(), 1);
        assert_eq!(observed.refunds, 0);
        assert!(observed.emitted.is_empty());
    }
}

#[test]
fn buffered_refusals_keep_their_original_classification() {
    for status in [400, 402, 403] {
        let mut config = adaptive_config();
        config.metered_fallback = false;
        let reply = Reply { streaming: false, status,
            result: Ok("request refused".into()) };
        let observed = run_with_payload(vec![reply], config, Vec::new(), adaptive_payload());
        let error = observed.result.unwrap_err();
        assert!(!error.starts_with(framer_core::error::ADAPTIVE_INCOMPLETE_PREFIX));
        if status == 402 { assert_eq!(error, "credits-exhausted"); }
        assert_eq!(observed.requests.len(), 1);
        assert_eq!(observed.refunds, 0);
        assert!(observed.emitted.is_empty());
    }
}
