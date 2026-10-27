use super::*;

#[test]
fn buffered_thinking_only_limit_finishes_without_empty_continuation() {
    for content in [vec![thought()], vec![json!({"type":"redacted_thinking","data":"opaque"})],
        vec![thought(), json!({"type":"text","text":""})],
        vec![], vec![json!({"type":"text","text":" \n\t"})]] {
        let observed = adaptive(vec![buffered_blocks(content.clone(), "max_tokens")], 2);
        assert_terminal(observed, content, 1, 0);
    }
}

#[test]
fn idle_fallback_thinking_limit_finishes_without_empty_continuation() {
    let observed = adaptive(vec![failed_request(), failed_request(),
        streamed(stream(thought(), "max_tokens"))], 2);
    assert_terminal(observed, vec![thought()], 3, 1);
}

#[test]
fn tool_fallback_thinking_limit_finishes_without_empty_continuation() {
    let observed = adaptive(vec![buffered(tool(), "max_tokens"),
        buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens"),
        streamed(stream(thought(), "max_tokens"))], 4);
    assert_terminal(observed, vec![thought()], 4, 1);
}

fn thought() -> Value {
    json!({"type":"thinking","thinking":"fixture thought","signature":"signed-fixture"})
}

#[test]
fn mixed_thinking_text_continues_with_nonempty_text_history() {
    let text = json!({"type":"text","text":"first"});
    let content = vec![thought(), text.clone()];
    let observed = adaptive(vec![buffered_blocks(content.clone(), "max_tokens"),
        buffered(json!({"type":"text","text":"last"}), "end_turn")], 2);
    let response = observed.result.as_ref().unwrap();
    assert_eq!(response.stop_reason, "end_turn");
    assert_eq!(response.output_tokens, 160);
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.requests[1]["request"]["messages"][1],
        json!({"role":"assistant","content":[text]}));
    assert_eq!(observed.emitted[..2], content);
    assert_eq!(response.content[0], thought());
}

fn adaptive(replies: Vec<Reply>, max_chunks: u32) -> Observed {
    run_with_payload(replies, ChainConfig { chunk_tokens:600, max_chunks,
        max_total_tokens:Some(4096), metered_fallback:true, ..ChainConfig::default() },
        Vec::new(), json!({"model":"claude-opus-5-5", "thinking":{"type":"adaptive"},
            "output_config":{"effort":"high"}, "speed":"fast",
            "messages":[{"role":"user","content":"fixture"}]}))
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
