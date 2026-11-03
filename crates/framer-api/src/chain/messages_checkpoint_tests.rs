use super::*;

fn thought() -> Value {
    json!({"type":"thinking","thinking":"fixture reasoning","signature":"fixture-signature"})
}

fn opus(replies: Vec<Reply>) -> Observed {
    run_with_payload(replies, ChainConfig { max_chunks:8, max_total_tokens:Some(4096),
        metered_fallback:false, ..ChainConfig::default() }, Vec::new(),
        json!({"model":"claude-opus-5", "thinking":{"type":"adaptive"},
            "output_config":{"effort":"high"}, "speed":"fast",
            "messages":[{"role":"user","content":"fixture request"}]}))
}

#[test]
fn signed_thinking_only_checkpoint_continues_without_losing_blocks() {
    let observed = opus(vec![buffered(thought(), "max_tokens"),
        buffered(json!({"type":"text","text":"answer"}), "end_turn")]);
    let response = observed.result.unwrap();
    assert_eq!(response.stop_reason, "end_turn");
    assert_eq!(response.chunks, 2);
    assert_eq!(response.content[0], thought());
    assert_eq!(observed.requests[0]["request"]["max_tokens"], 600);
    assert_eq!(observed.requests[1]["request"]["messages"][1]["content"], json!([thought()]));
    for request in &observed.requests {
        assert_eq!(request["request"]["thinking"]["type"], "adaptive");
        assert_eq!(request["request"]["output_config"]["effort"], "high");
        assert_eq!(request["request"]["speed"], "fast");
        assert_eq!(request["request"]["stream"], false);
    }
}

#[test]
fn missing_signature_stops_before_publication_or_another_request() {
    let observed = opus(vec![buffered(json!({"type":"thinking","thinking":"unfinished"}), "max_tokens")]);
    assert!(observed.result.unwrap_err().contains("complete signed thinking block"));
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.requests.len(), 1);
}

#[test]
fn known_truncated_tool_keeps_checkpoint_without_publishing_partial_input() {
    let first = Reply { streaming:false, status:200, result:Ok(json!({
        "id":"cut-tool", "content":[thought(), tool()], "stop_reason":"max_tokens",
        "usage":{"input_tokens":10,"output_tokens":600}}).to_string()) };
    let observed = opus(vec![first, buffered(tool(), "tool_use")]);
    let response = observed.result.unwrap();
    assert_eq!(response.stop_reason, "tool_use");
    assert_eq!(response.content, vec![thought(), tool()]);
    assert_eq!(response.output_tokens, 680);
    assert_eq!(observed.emitted, response.content);
    assert_eq!(observed.requests[1]["request"]["messages"][1]["content"], json!([thought()]));
    assert_eq!(observed.refunds, 0);
}

#[test]
fn unknown_completion_after_checkpoint_never_replays() {
    let failure = Reply { streaming:false, status:504, result:Ok("gateway timeout".into()) };
    let observed = opus(vec![buffered(thought(), "max_tokens"), failure]);
    let error = observed.result.unwrap_err();
    assert!(error.contains("completion is unknown"));
    assert!(error.contains("gateway timeout"));
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.emitted, vec![thought()]);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn refusal_after_checkpoint_retains_the_provider_explanation() {
    let details = json!({"type":"refusal","category":"fixture-policy","explanation":"fixture cause"});
    let refusal = Reply { streaming:false, status:200, result:Ok(json!({
        "id":"refused", "content":[], "stop_reason":"refusal", "stop_details":details,
        "usage":{"output_tokens":1}}).to_string()) };
    let observed = opus(vec![buffered(thought(), "max_tokens"), refusal]);
    let result = observed.result.unwrap();
    assert_eq!(result.stop_reason, "refusal");
    assert_eq!(result.stop_details, Some(details));
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.refunds, 0);
}

#[tokio::test(start_paused = true)]
async fn client_followup_sends_original_upstream_turns() {
    let account = AccountId(uuid::Uuid::from_u128(1));
    let session = Session { active_user: FramerUserId("fixture-user".into()),
        cookie_header: "fixture-only".into() };
    let transport = Transport::with_base("http://127.0.0.1:9".into());
    let cache = Mutex::new(vec![(account,
        AiToken::for_session(&session, "fixture-only".into(), i64::MAX))]);
    let browser = Fixture { replies: Mutex::new(vec![
        buffered(thought(), "max_tokens"),
        buffered(json!({"type":"text","text":"answer"}), "end_turn"),
        buffered(json!({"type":"text","text":"followup"}), "end_turn")].into()),
        requests: Mutex::new(Vec::new()), refunds: AtomicUsize::new(0),
        delays: Mutex::new(VecDeque::new()) };
    let config = ChainConfig { max_total_tokens: Some(4096), metered_fallback: false,
        ..ChainConfig::default() };
    let mut payload = json!({"model":"claude-opus-5","thinking":{"type":"adaptive"},
        "messages":[{"role":"user","content":"original"}]});
    let project = ProjectId("fixture-project".into());
    let first = complete_long(&transport, &cache, &session, &browser, &account,
        &project, payload.clone(), &config, "fixture-chat", None).await.unwrap();
    payload["messages"].as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":first.content}),
        json!({"role":"user","content":"follow up"})]);
    complete_long(&transport, &cache, &session, &browser, &account,
        &project, payload, &config, "fixture-chat", None).await.unwrap();
    let requests = browser.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let history = &requests[2]["request"]["messages"];
    assert_eq!(history.as_array().unwrap().len(), 5);
    assert_eq!(history[1]["content"], json!([thought()]));
    assert_eq!(history[2]["content"], checkpoint::CONTINUE);
    assert_eq!(history[3]["content"][0]["text"], "answer");
}
