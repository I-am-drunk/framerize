use super::*;

#[test]
fn empty_checkpoint_finishes_without_empty_continuation() {
    for content in [vec![], vec![json!({"type":"text","text":" \n\t"})]] {
        let observed = adaptive(vec![buffered_blocks(content.clone(), "max_tokens")], 2);
        assert_terminal(observed, content, 1, 0);
    }
}

#[test]
fn adaptive_transport_failure_does_not_retry_or_spend() {
    let observed = adaptive(vec![failed_request()], 2);
    let error = observed.result.as_ref().unwrap_err();
    assert!(error.contains("fixture transport closed"));
    assert_eq!(observed.refunds, 0);
    assert_eq!(observed.requests.len(), 1);
    assert_eq!(observed.requests[0]["request"]["stream"], false);
    assert!(observed.emitted.is_empty());
}

#[test]
fn acknowledged_truncated_tool_publishes_only_the_complete_attempt() {
    let observed = adaptive(vec![buffered(tool(), "max_tokens"),
        buffered(tool(), "tool_use")], 4);
    assert_eq!(observed.result.unwrap().stop_reason, "tool_use");
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.emitted, vec![tool()]);
    assert_eq!(observed.requests[0]["request"]["messages"],
        observed.requests[1]["request"]["messages"]);
}

fn thought() -> Value {
    json!({"type":"thinking","thinking":"fixture thought","signature":"signed-fixture"})
}

#[test]
fn adaptive_continuation_preserves_signed_blocks_and_returns_the_finished_answer() {
    let text = json!({"type":"text","text":"first"});
    let content = vec![thought(), text.clone()];
    let observed = adaptive(vec![buffered_blocks(content.clone(), "max_tokens"),
        buffered(json!({"type":"text","text":"last"}), "end_turn")], 2);
    let response = observed.result.as_ref().unwrap();
    assert_eq!(response.stop_reason, "end_turn");
    assert_eq!(response.output_tokens, 160);
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.requests[1]["request"]["messages"][1]["content"], json!(content));
    assert_eq!(response.content[1]["text"], "firstlast");
    assert_eq!(response.content[0], thought());
}

fn adaptive(replies: Vec<Reply>, max_chunks: u32) -> Observed {
    adaptive_delayed(replies, max_chunks, Vec::new())
}

fn adaptive_delayed(replies: Vec<Reply>, max_chunks: u32,
    delays: Vec<std::time::Duration>) -> Observed {
    run_with_payload(replies, ChainConfig { chunk_tokens:600, max_chunks,
        max_total_tokens:Some(4096), metered_fallback:false, ..ChainConfig::default() },
        delays, json!({"model":"claude-sonnet-5-5", "thinking":{"type":"adaptive"},
            "output_config":{"effort":"high"}, "speed":"fast",
            "messages":[{"role":"user","content":"fixture"}]}))
}

#[test]
fn slow_adaptive_terminal_preserves_the_upstream_result() {
    let text = json!({"type":"text","text":"first"});
    let observed = adaptive_delayed(vec![buffered_blocks(
        vec![text.clone(), thought()], "end_turn")], 3,
        vec![std::time::Duration::from_secs(12)]);
    let response = observed.result.as_ref().unwrap();
    assert_eq!(response.stop_reason, "end_turn");
    assert_eq!(response.output_tokens, 80);
    assert_eq!(response.content, vec![text, thought()]);
    assert_eq!(observed.emitted, response.content);
    assert_eq!(observed.requests.len(), 1);
}

fn buffered_blocks(content: Vec<Value>, stop: &str) -> Reply {
    Reply { streaming:false, status:200, result:Ok(json!({
        "id":"fixture-thinking", "content":content, "stop_reason":stop,
        "usage":{"input_tokens":19,"output_tokens":80},
    }).to_string()) }
}

fn assert_terminal(observed: Observed, content: Vec<Value>, calls: usize, metered: u32) {
    let response = observed.result.as_ref().unwrap();
    assert_eq!(response.content, content);
    assert_eq!(response.stop_reason, "max_tokens");
    assert_eq!(response.output_tokens, 80);
    assert_eq!(response.metered_chunks, metered);
    assert_eq!(observed.emitted, content);
    assert_eq!(observed.requests.len(), calls);
    let client = super::super::stream::parse(&(observed.frames.join("\n\n") + "\n\n")).unwrap();
    assert_eq!(client.content, content);
    assert_eq!(client.stop_reason, "max_tokens");
    for request in observed.requests {
        assert_eq!(request["request"]["output_config"]["effort"], "high");
        assert_eq!(request["request"]["thinking"]["type"], "adaptive");
        assert_eq!(request["request"]["speed"], "fast");
    }
}
