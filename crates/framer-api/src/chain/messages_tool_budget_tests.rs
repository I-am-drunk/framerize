use super::*;

fn config() -> ChainConfig {
    ChainConfig { max_chunks: 6, max_total_tokens: Some(16_000),
        metered_fallback: false, ..ChainConfig::default() }
}

fn reply(content: Vec<Value>, stop: &str, tokens: u64) -> Reply {
    Reply { streaming: false, status: 200, result: Ok(json!({
        "id":"tool-budget-fixture", "content":content, "stop_reason":stop,
        "usage":{"input_tokens":19,"output_tokens":tokens}}).to_string()) }
}

fn limits(observed: &Observed) -> Vec<u64> {
    observed.requests.iter().map(|r| r["request"]["max_tokens"].as_u64().unwrap()).collect()
}

fn partial() -> Value {
    let mut call = tool();
    call["input"] = Value::Null;
    call
}

#[test]
fn complete_tool_can_exceed_the_prose_checkpoint_ceiling() {
    let mut call = tool();
    call["input"] = json!({"values":(0..700).collect::<Vec<_>>()});
    let observed = run_with_config(vec![
        reply(vec![partial()], "max_tokens", 600),
        reply(vec![partial()], "max_tokens", 1200),
        reply(vec![call.clone()], "tool_use", 2100),
    ], config());
    assert_eq!(limits(&observed), [600, 1200, 2400]);
    let response = observed.result.unwrap();
    assert_eq!(response.content, vec![call.clone()]);
    assert_eq!(observed.emitted, vec![call]);
    assert_eq!(response.output_tokens, 3900);
    assert_eq!(response.metered_chunks, 0);
    assert_eq!(observed.refunds, 0);
    for request in &observed.requests[1..] {
        assert_eq!(request["request"]["messages"], observed.requests[0]["request"]["messages"]);
    }
}

#[test]
fn tool_retries_grow_after_a_preserved_signed_prefix() {
    let thought = json!({"type":"thinking","thinking":"fixture","signature":"fixture-signature"});
    let observed = run_with_config(vec![
        reply(vec![thought.clone()], "max_tokens", 600),
        reply(vec![thought.clone(), partial()], "max_tokens", 1200),
        reply(vec![partial()], "max_tokens", 2400),
        reply(vec![tool()], "tool_use", 3000),
    ], config());
    assert_eq!(limits(&observed), [600, 1200, 2400, 4800]);
    assert_eq!(observed.emitted, vec![thought.clone(), thought.clone(), tool()]);
    let history = &observed.requests[2]["request"]["messages"];
    assert_eq!(history[3]["content"], json!([thought]));
    assert_eq!(history[4]["content"], checkpoint::TOOL_CONTINUE);
    assert_eq!(*history, observed.requests[3]["request"]["messages"]);
    assert_eq!(observed.result.unwrap().output_tokens, 7200);
}

#[test]
fn larger_retry_still_respects_the_remaining_client_budget() {
    let observed = run_with_config(vec![
        reply(vec![partial()], "max_tokens", 600),
        reply(vec![partial()], "max_tokens", 1200),
        reply(vec![tool()], "tool_use", 2100),
    ], ChainConfig { max_total_tokens: Some(4000), ..config() });
    assert_eq!(limits(&observed), [600, 1200, 2200]);
    assert_eq!(observed.result.unwrap().output_tokens, 3900);
}

#[test]
fn incomplete_tools_stop_after_two_larger_retries_even_without_a_total() {
    let observed = run_with_config(vec![
        reply(vec![partial()], "max_tokens", 600),
        reply(vec![partial()], "max_tokens", 1200),
        reply(vec![partial()], "max_tokens", 2400),
    ], ChainConfig { max_total_tokens: None, ..config() });
    assert_eq!(limits(&observed), [600, 1200, 2400]);
    assert!(observed.result.unwrap_err().contains("complete tool input is unavailable"));
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.refunds, 0);
}

#[test]
fn tool_retry_respects_explicit_thinking_floor_without_changing_settings() {
    let thinking = json!({"type":"enabled","budget_tokens":4096});
    let observed = run_with_payload(vec![
        reply(vec![partial()], "max_tokens", 4097),
        reply(vec![tool()], "tool_use", 6000),
    ], config(), Vec::new(), json!({"model":"claude-opus-5-5",
        "thinking":thinking, "output_config":{"effort":"high"},
        "messages":[{"role":"user","content":"fixture"}]}));
    assert_eq!(limits(&observed), [4097, 8194]);
    assert_eq!(observed.result.unwrap().output_tokens, 10097);
    for request in observed.requests {
        assert_eq!(request["request"]["thinking"], thinking);
        assert_eq!(request["request"]["output_config"]["effort"], "high");
    }
}

#[test]
fn larger_tool_retry_never_replays_unknown_completion() {
    let observed = run_with_config(vec![
        reply(vec![partial()], "max_tokens", 600),
        reply(vec![partial()], "max_tokens", 1200), failed_request(),
    ], config());
    assert_eq!(limits(&observed), [600, 1200, 2400]);
    assert!(observed.result.unwrap_err().contains("completion is unknown"));
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.refunds, 0);
}

#[test]
fn completed_retry_returns_to_the_ordinary_checkpoint_budget() {
    let text = json!({"type":"text","text":"fixture checkpoint"});
    let observed = run_with_config(vec![
        reply(vec![partial()], "max_tokens", 600),
        reply(vec![partial()], "max_tokens", 1200),
        reply(vec![text.clone()], "max_tokens", 2400),
        reply(vec![text.clone()], "end_turn", 500),
    ], config());
    assert_eq!(limits(&observed), [600, 1200, 2400, 1200]);
    assert_eq!(observed.emitted, vec![text.clone(), text]);
    assert!(observed.result.is_ok());
}

#[test]
fn exhausted_tool_budget_stops_without_publishing_or_dispatching() {
    let observed = run_with_config(vec![
        reply(vec![partial()], "max_tokens", 600),
        reply(vec![partial()], "max_tokens", 1200),
    ], ChainConfig { max_total_tokens: Some(1800), ..config() });
    assert_eq!(limits(&observed), [600, 1200]);
    assert!(observed.result.unwrap_err().contains("exhausted the requested output allowance"));
    assert!(observed.emitted.is_empty());
}
