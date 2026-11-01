use super::*;

fn payload(budget: Value) -> Value {
    json!({"model":"claude-opus-5-5","speed":"fast",
        "thinking":{"type":"enabled","budget_tokens":budget},
        "output_config":{"effort":"high"},
        "messages":[{"role":"user","content":"fixture"}]})
}

fn configured(replies: Vec<Reply>, budget: Value, total: Option<u32>, fallback: bool) -> Observed {
    run_with_payload(replies, ChainConfig { chunk_tokens:600, max_chunks:4,
        max_total_tokens:total, metered_fallback:fallback, ..ChainConfig::default() },
        Vec::new(), payload(budget))
}

fn text() -> Value { json!({"type":"text","text":"fixture output"}) }

fn assert_requests(observed: &Observed, budget: u32, expected: &[u32]) {
    assert_eq!(observed.requests.len(), expected.len());
    for (request, amount) in observed.requests.iter().zip(expected) {
        let request = &request["request"];
        assert_eq!(request["max_tokens"], *amount);
        assert_eq!(request["thinking"]["budget_tokens"], budget);
        assert_eq!(request["speed"], "fast");
        assert_eq!(request["output_config"]["effort"], "high");
    }
}

#[test]
fn initial_requests_cover_the_preserved_thinking_budget() {
    for budget in [1024, 4096] {
        for total in [Some(budget + 1), None] {
            let observed = configured(vec![buffered(text(), "end_turn")], json!(budget), total, false);
            assert!(observed.result.is_ok());
            assert_requests(&observed, budget, &[budget + 1]);
        }
    }
}

#[test]
fn invalid_budget_or_insufficient_initial_total_never_dispatches() {
    for budget in [Value::Null, json!(1023), json!(1024.5), json!("1024"), json!(-1), json!(4294967295_u64)] {
        let observed = configured(vec![], budget, Some(8192), false);
        assert!(observed.result.unwrap_err().starts_with("invalid enabled thinking budget"));
        assert!(observed.requests.is_empty());
    }
    let observed = configured(vec![], json!(1024), Some(1024), false);
    assert!(observed.result.unwrap_err().contains("cannot accommodate"));
    assert!(observed.requests.is_empty());
}

#[test]
fn transport_retry_and_opted_in_fallback_keep_the_budget_floor() {
    let observed = configured(vec![failed_request(), failed_request(),
        streamed(stream(text(), "end_turn"))], json!(1024), Some(4096), true);
    assert!(observed.result.is_ok());
    assert_requests(&observed, 1024, &[1025, 1025, 1025]);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn exhausted_remaining_thinking_allowance_preserves_terminal_output() {
    for fallback in [false, true] {
        let replies = if fallback { vec![failed_request(), failed_request(),
            streamed(stream(text(), "max_tokens"))] } else { vec![buffered(text(), "max_tokens")] };
        let observed = configured(replies, json!(1024), Some(1100), fallback);
        let response = observed.result.as_ref().unwrap();
        assert_eq!(response.stop_reason, "max_tokens");
        assert_eq!(response.output_tokens, 80);
        assert_eq!(response.content, vec![text()]);
        assert_eq!(observed.emitted, vec![text()]);
        let client = super::super::stream::parse(&(observed.frames.join("\n\n") + "\n\n")).unwrap();
        assert_eq!(client.stop_reason, "max_tokens");
        assert_requests(&observed, 1024, &vec![1025; if fallback { 3 } else { 1 }]);
    }
}

#[test]
fn tool_retries_and_uncapped_stream_respect_large_thinking_budget() {
    let observed = configured(vec![buffered(tool(), "max_tokens"),
        buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens"),
        streamed(stream(text(), "end_turn"))], json!(9000), None, true);
    assert!(observed.result.is_ok());
    assert_requests(&observed, 9000, &[9001, 9001, 9001, 9001]);
}

#[test]
fn thinking_floor_does_not_enable_metered_fallback() {
    let observed = configured(vec![failed_request(), failed_request()], json!(1024), Some(4096), false);
    assert!(observed.result.as_ref().unwrap_err().contains("fallback off"));
    assert_requests(&observed, 1024, &[1025, 1025]);
}

#[test]
fn tool_fallback_stops_when_remaining_allowance_is_below_floor() {
    let observed = configured(vec![buffered(tool(), "max_tokens"),
        buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens"),
        streamed(stream(text(), "max_tokens"))], json!(1024), Some(1100), true);
    let response = observed.result.as_ref().unwrap();
    assert_eq!(response.stop_reason, "max_tokens");
    assert_eq!(response.output_tokens, 80);
    assert_eq!(response.content, vec![text()]);
    assert_requests(&observed, 1024, &[1025, 1100, 1100, 1100]);
}

#[test]
fn adaptive_thinking_keeps_the_requested_output_limit() {
    let mut request = payload(Value::Null);
    request["thinking"] = json!({"type":"adaptive"});
    let observed = run_with_payload(vec![buffered(text(), "end_turn")],
        ChainConfig { chunk_tokens:600, max_chunks:1,
            max_total_tokens:Some(4096), ..ChainConfig::default() }, Vec::new(), request);
    assert!(observed.result.is_ok());
    assert_eq!(observed.requests[0]["request"]["max_tokens"], 4096);
    assert_eq!(observed.requests[0]["request"]["thinking"], json!({"type":"adaptive"}));
    assert_eq!(observed.requests[0]["request"]["output_config"]["effort"], "high");
}

#[test]
fn adaptive_small_and_large_limits_preserve_effort_and_fast_mode() {
    for total in [256, 128000] {
        for effort in ["low", "medium", "high", "xhigh"] {
            let mut request = payload(Value::Null);
            request["thinking"] = json!({"type":"adaptive"});
            request["output_config"]["effort"] = json!(effort);
            let observed = run_with_payload(vec![buffered(text(), "end_turn")],
                ChainConfig { max_total_tokens:Some(total),
                    ..ChainConfig::default() }, Vec::new(), request);
            assert!(observed.result.is_ok());
            assert_eq!(observed.requests.len(), 1);
            let sent = &observed.requests[0]["request"];
            assert_eq!(sent["max_tokens"], total);
            assert_eq!(sent["thinking"]["type"], "adaptive");
            assert_eq!(sent["output_config"]["effort"], effort);
            assert_eq!(sent["speed"], "fast");
            assert_eq!(sent["stream"], false);
        }
    }
}

#[test]
fn adaptive_without_a_total_uses_the_configured_request_allowance() {
    let mut request = payload(Value::Null);
    request["thinking"] = json!({"type":"adaptive"});
    let observed = run_with_payload(vec![buffered(text(), "end_turn")],
        ChainConfig { chunk_tokens:2048, max_total_tokens:None,
            ..ChainConfig::default() }, Vec::new(), request);
    assert!(observed.result.is_ok());
    assert_eq!(observed.requests[0]["request"]["max_tokens"], 2048);
}

#[test]
fn cancelled_adaptive_request_never_dispatches() {
    let mut request = payload(Value::Null);
    request["thinking"] = json!({"type":"adaptive"});
    let observed = run_with_payload(vec![],
        ChainConfig { max_total_tokens:Some(128000),
            cancelled:Some(std::sync::Arc::new(|| true)),
            ..ChainConfig::default() }, Vec::new(), request);
    assert!(observed.result.unwrap_err().contains("cancelled"));
    assert!(observed.requests.is_empty());
    assert!(observed.emitted.is_empty());
}
