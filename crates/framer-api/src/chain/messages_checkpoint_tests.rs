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
