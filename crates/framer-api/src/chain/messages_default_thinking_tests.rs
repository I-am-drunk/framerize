use super::*;

fn payload(model: &str) -> Value {
    json!({"model":model,"speed":"fast","output_config":{"effort":"high"},
        "messages":[{"role":"user","content":"fixture"}]})
}

fn config() -> ChainConfig {
    ChainConfig { chunk_tokens:600, max_chunks:4, max_total_tokens:Some(2400),
        metered_fallback:false, ..ChainConfig::default() }
}

fn text() -> Value { json!({"type":"text","text":"fixture output"}) }

#[test]
fn default_thinking_uses_checkpoints_without_inventing_settings() {
    for model in ["claude-fable-5-1", "claude-fable-5.1",
        "anthropic/claude-fable-5-1", "anthropic/claude-fable-5.1"] {
        let request = payload(model);
        let observed = run_with_payload(vec![buffered(text(), "end_turn")],
            config(), Vec::new(), request.clone());
        assert!(observed.result.is_ok());
        assert_eq!(observed.requests.len(), 1);
        let sent = &observed.requests[0]["request"];
        assert_eq!(sent["max_tokens"], 600);
        assert!(sent.get("thinking").is_none());
        assert_eq!(sent["output_config"], request["output_config"]);
        assert_eq!(sent["speed"], request["speed"]);
    }
}

#[test]
fn explicit_disabled_thinking_keeps_text_continuation() {
    let mut request = payload("anthropic/claude-fable-5.1");
    request["thinking"] = json!({"type":"disabled"});
    let observed = run_with_payload(vec![buffered(text(), "max_tokens"),
        buffered(text(), "end_turn")], config(), Vec::new(), request);
    assert!(observed.result.is_ok());
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.requests[0]["request"]["max_tokens"], 600);
    for sent in observed.requests {
        assert_eq!(sent["request"]["thinking"]["type"], "disabled");
    }
}

#[test]
fn default_thinking_transport_failure_never_replays_or_spends() {
    let observed = run_with_payload(vec![failed_request()], config(),
        Vec::new(), payload("claude-fable-5-1"));
    assert!(observed.result.is_err());
    assert_eq!(observed.requests.len(), 1);
    assert_eq!(observed.requests[0]["request"]["max_tokens"], 600);
    assert_eq!(observed.refunds, 0);
    assert!(observed.emitted.is_empty());
}

#[test]
fn default_thinking_continues_from_complete_signed_reasoning() {
    let thought = json!({"type":"thinking","thinking":"fixture","signature":"fixture"});
    let observed = run_with_payload(vec![buffered(thought.clone(), "max_tokens"),
        buffered(text(), "end_turn")],
        config(), Vec::new(), payload("claude-fable-5-1"));
    let result = observed.result.unwrap();
    assert_eq!(result.stop_reason, "end_turn");
    assert_eq!(result.content, vec![thought.clone(), text()]);
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.requests[1]["request"]["messages"][1]["content"], json!([thought]));
}

#[test]
fn default_thinking_obeys_small_and_uncapped_request_allowances() {
    for (total, expected) in [(Some(128),128), (None,600)] {
        let mut limits = config();
        limits.chunk_tokens = 2048;
        limits.max_total_tokens = total;
        let observed = run_with_payload(vec![buffered(text(), "end_turn")],
            limits, Vec::new(), payload("anthropic/claude-fable-5.1"));
        assert!(observed.result.is_ok());
        assert_eq!(observed.requests[0]["request"]["max_tokens"], expected);
    }
}

#[test]
fn default_thinking_gateway_timeout_reports_mode_cause_and_no_replay() {
    let reply = Reply { streaming:false, status:504,
        result:Ok("upstream execution status unknown".into()) };
    let observed = run_with_payload(vec![reply], config(),
        vec![std::time::Duration::from_secs(61)], payload("claude-fable-5-1"));
    let error = observed.result.unwrap_err();
    assert!(error.contains("default reasoning"));
    assert!(error.contains("600 output tokens"));
    assert!(error.contains("504 on /ai/v3/messages/"));
    assert!(error.contains("upstream execution status unknown"));
    assert!(error.starts_with(framer_core::error::ADAPTIVE_INCOMPLETE_PREFIX));
    assert_eq!(observed.requests.len(), 1);
    assert_eq!(observed.refunds, 0);
}
