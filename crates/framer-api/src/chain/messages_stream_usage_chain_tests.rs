use super::*;

fn with_usage(initial: Value, terminal: Value) -> Reply {
    let raw = stream(tool(), "tool_use");
    let mut values: Vec<Value> = raw.lines().filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).unwrap()).collect();
    for event in &mut values {
        if event["type"] == "message_start" { event["message"]["usage"] = initial.clone(); }
        if event["type"] == "message_delta" { event["usage"] = terminal.clone(); }
    }
    streamed(values.iter().map(|event| format!("data: {event}\n\n")).collect())
}

#[test]
fn opted_stream_rejects_malformed_usage_before_emitting_tools() {
    let observed = run_opted_stream(vec![
        with_usage(json!({"input_tokens":17}), json!({"output_tokens":"80"}))], 1);
    assert!(observed.result.unwrap_err().contains("usage metadata"));
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.requests.len(), 1);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn tool_fallback_rejects_malformed_usage_before_emitting_tools() {
    let observed = run(vec![buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens"),
        with_usage(json!(null), json!({"output_tokens":80}))], 3);
    assert!(observed.result.unwrap_err().contains("usage metadata"));
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.requests.len(), 3);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn streaming_paths_preserve_streamed_usage_metadata() {
    for tool_repair in [false, true] {
        let mut replies = if tool_repair {
            vec![buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens")]
        } else { Vec::new() };
        replies.push(with_usage(json!({"input_tokens":17,"cache_read_input_tokens":4}),
            json!({"output_tokens":80,"server_tool_use":{"web_search_requests":1}})));
        let observed = if tool_repair { run(replies, 3) } else { run_opted_stream(replies, 1) };
        let response = observed.result.unwrap();
        assert_eq!(response.usage, json!({"input_tokens":if tool_repair { 19 } else { 17 },
            "output_tokens":if tool_repair { 240 } else { 80 },"cache_read_input_tokens":4,
            "server_tool_use":{"web_search_requests":1}}));
        assert_eq!(response.content, vec![tool()]);
        assert_eq!(response.metered_chunks, 1);
        assert_eq!(observed.emitted, vec![tool()]);
        assert_eq!(observed.requests.len(), if tool_repair { 3 } else { 1 });
        assert_eq!(observed.refunds, 0);
    }
}

#[test]
fn streaming_paths_accept_nullable_delta_usage() {
    for tool_repair in [false, true] {
        let mut replies = if tool_repair {
            vec![buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens")]
        } else { Vec::new() };
        replies.push(with_usage(json!({"input_tokens":17,"cache_read_input_tokens":4}),
            json!({"input_tokens":null,"output_tokens":80,"cache_read_input_tokens":null})));
        let observed = if tool_repair { run(replies, 3) } else { run_opted_stream(replies, 1) };
        let response = observed.result.unwrap();
        assert_eq!(response.usage, json!({"input_tokens":if tool_repair { 19 } else { 17 },
            "output_tokens":if tool_repair { 240 } else { 80 },"cache_read_input_tokens":4}));
        assert_eq!(observed.emitted, vec![tool()]);
        assert_eq!(observed.requests.len(), if tool_repair { 3 } else { 1 });
        assert_eq!(observed.refunds, 0);
    }
}
