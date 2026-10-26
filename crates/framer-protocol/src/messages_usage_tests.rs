use super::*;

fn usage() -> Value {
    json!({"input_tokens":37,"output_tokens":81,"cache_creation_input_tokens":5,
        "cache_read_input_tokens":7,"server_tool_use":{"web_search_requests":1},
        "output_tokens_details":{"thinking_tokens":3}})
}

fn response() -> AssembledResponse {
    let mut full_usage = usage();
    full_usage["service_tier"] = json!("standard");
    full_usage["cache_creation"] = json!({"ephemeral_5m_input_tokens":5});
    AssembledResponse { id:"msg-fixture".into(),
        content:vec![json!({"type":"text","text":"answer"})],
        stop_reason:"stop_sequence".into(), stop_sequence:Some("END".into()),
        usage:full_usage, input_tokens:37, output_tokens:81, chunks:2, metered_chunks:0 }
}

fn values(frames: Vec<String>) -> Vec<Value> {
    frames.iter().map(|frame| serde_json::from_str(frame.lines()
        .find_map(|line| line.strip_prefix("data: ")).unwrap()).unwrap()).collect()
}

#[test]
fn complete_messages_sse_preserves_initial_and_final_usage() {
    let response = response();
    let events = values(render_sse(ClientDialect::AnthropicMessages, &response, "fixture", false));
    let mut expected_start = response.usage.clone();
    expected_start["output_tokens"] = json!(0);
    assert_eq!(events[0]["message"]["usage"], expected_start);
    let delta = events.iter().find(|event| event["type"] == "message_delta").unwrap();
    assert_eq!(delta["usage"], usage());
    assert_eq!(delta["delta"], json!({"stop_reason":"stop_sequence","stop_sequence":"END"}));
}

#[test]
fn incremental_messages_sse_updates_input_and_optional_usage() {
    let response = response();
    let dialect = ClientDialect::AnthropicMessages;
    let mut frames = stream_open(dialect, "fixture", "msg-fixture");
    let mut composer = SseComposer::new(dialect, "msg-fixture", "fixture");
    frames.extend(composer.push_chunk(&response.content));
    frames.extend(composer.finish(&response, "fixture", false));
    let events = values(frames);
    assert_eq!(events[0]["message"]["usage"], json!({"input_tokens":0,"output_tokens":0}));
    let delta = events.iter().find(|event| event["type"] == "message_delta").unwrap();
    assert_eq!(delta["usage"], usage());
    assert_eq!(delta["delta"], json!({"stop_reason":"stop_sequence","stop_sequence":"END"}));
    assert_eq!(events.last().unwrap()["type"], "message_stop");
}
