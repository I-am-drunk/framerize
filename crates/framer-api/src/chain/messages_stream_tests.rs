use super::*;

#[path = "messages_citation_tests.rs"]
mod citations;

#[path = "messages_block_validation_tests.rs"]
mod validation;

#[path = "messages_stream_usage_tests.rs"]
mod usage;

#[path = "messages_signature_tests.rs"]
mod signatures;

#[path = "messages_metadata_tests.rs"]
mod metadata;

#[path = "messages_sse_event_tests.rs"]
mod sse_events;

#[path = "messages_line_endings_tests.rs"]
mod line_endings;

fn events(block: Value, deltas: Vec<Value>, reason: &str) -> Vec<Value> {
    let mut events = vec![
        json!({"type":"message_start","message":{"id":"msg-fixture",
            "usage":{"input_tokens":17}}}),
        json!({"type":"content_block_start","index":0,"content_block":block}),
    ];
    events.extend(deltas.into_iter().map(|delta|
        json!({"type":"content_block_delta","index":0,"delta":delta})));
    events.extend([
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":reason,
            "stop_sequence":null},"usage":{"output_tokens":9}}),
        json!({"type":"message_stop"}),
    ]);
    events
}

fn encode(events: &[Value]) -> String {
    events.iter().map(|event| format!("data: {event}\n\n")).collect()
}

fn text_events(reason: &str) -> Vec<Value> {
    events(json!({"type":"text","text":"prefix"}),
        vec![json!({"type":"text_delta","text":" suffix"})], reason)
}

#[test]
fn complete_text_preserves_stop_sequence_and_usage() {
    let mut values = text_events("stop_sequence");
    values[4]["delta"]["stop_sequence"] = json!("END");
    let response = parse(&encode(&values)).unwrap();
    assert_eq!(response.id, "msg-fixture");
    assert_eq!(response.content, vec![json!({"type":"text","text":"prefix suffix"})]);
    assert_eq!(response.stop_reason, "stop_sequence");
    assert_eq!(response.stop_sequence.as_deref(), Some("END"));
    assert_eq!((response.input_tokens, response.output_tokens), (17, 9));
    assert_eq!(response.usage, json!({"input_tokens":17,"output_tokens":9}));
}

#[test]
fn tool_json_deltas_preserve_identity_and_metadata() {
    let block = json!({"type":"tool_use","id":"tool-fixture","name":"fixture",
        "input":{},"extra":"preserved"});
    let values = events(block.clone(), vec![
        json!({"type":"input_json_delta","partial_json":"{\"value\":"}),
        json!({"type":"input_json_delta","partial_json":"42}"}),
    ], "tool_use");
    let response = parse(&encode(&values)).unwrap();
    let mut expected = block;
    expected["input"] = json!({"value":42});
    assert_eq!(response.content, vec![expected]);
    assert_eq!(response.stop_reason, "tool_use");
}

#[test]
fn complete_input_without_deltas_is_not_replaced() {
    for kind in ["tool_use", "server_tool_use"] {
        let block = json!({"type":kind,"id":"tool-fixture","name":"fixture",
            "input":{"value":42}});
        let response = parse(&encode(&events(block.clone(), vec![], "tool_use"))).unwrap();
        assert_eq!(response.content, vec![block]);
    }
}

#[test]
fn thinking_and_signature_deltas_are_preserved() {
    let values = events(json!({"type":"thinking","thinking":"","signature":""}),
        vec![json!({"type":"thinking_delta","thinking":"thought"}),
            json!({"type":"signature_delta","signature":"first"}),
            json!({"type":"signature_delta","signature":"second"})], "end_turn");
    assert_eq!(parse(&encode(&values)).unwrap().content,
        vec![json!({"type":"thinking","thinking":"thought","signature":"second"})]);
}

#[test]
fn truncated_stream_requires_message_stop() {
    let mut values = text_events("end_turn");
    values.pop();
    assert!(parse(&encode(&values)).unwrap_err().contains("without message_stop"));
    assert!(parse("").unwrap_err().contains("without message_stop"));
}

#[test]
fn message_stop_requires_closed_content_and_a_stop_reason() {
    for omitted in [3, 4] {
        let mut values = text_events("end_turn");
        values.remove(omitted);
        assert!(parse(&encode(&values)).unwrap_err().contains("before content completed"));
    }
}

#[test]
fn upstream_error_reports_its_cause_without_credentials_or_payload() {
    for position in [0, 2, 6] {
        let mut values = text_events("end_turn");
        values.insert(position, json!({"type":"error",
            "error":{"type":"overloaded_error","code":529,
                "message":"provider overloaded; Bearer secret-value",
                "request":{"private":"fixture payload"}}}));
        let error = parse(&encode(&values)).unwrap_err();
        assert!(error.starts_with("upstream 529 on /ai/v3/messages/:"));
        assert!(error.contains("provider overloaded"));
        assert!(error.contains("529"));
        assert!(!error.contains("secret-value"));
        assert!(!error.contains("fixture payload"));
    }
}

#[test]
fn malformed_json_cannot_be_skipped_before_a_valid_stop() {
    let values = text_events("end_turn");
    let stream = encode(&values[..2]) + "data: {broken\n\n" + &encode(&values[2..]);
    let error = parse(&stream).unwrap_err();
    assert!(error.contains("malformed event JSON"));
    assert!(!error.contains("broken"));
}

#[test]
fn invalid_tool_json_never_becomes_an_executable_raw_object() {
    for partial in ["{\"private fixture\":", "[]", "null"] {
        let values = events(json!({"type":"tool_use","id":"tool-fixture",
            "name":"fixture","input":{}}),
            vec![json!({"type":"input_json_delta","partial_json":partial})], "tool_use");
        let error = parse(&encode(&values)).unwrap_err();
        assert!(error.contains("tool input"));
        assert!(!error.contains("private fixture"));
    }
}

#[test]
fn token_limit_keeps_text_continuation_but_withholds_tools() {
    let response = parse(&encode(&text_events("max_tokens"))).unwrap();
    assert_eq!(response.stop_reason, "max_tokens");
    assert_eq!(response.content[0]["text"], "prefix suffix");
    let values = events(json!({"type":"tool_use","id":"tool-fixture",
        "name":"fixture","input":{}}), vec![], "max_tokens");
    assert!(parse(&encode(&values)).unwrap_err().contains("incomplete tool call"));
}

#[test]
fn content_events_cannot_invent_reopen_or_replace_blocks() {
    let original = text_events("end_turn");
    for index in [1, 2] {
        let mut values = original.clone();
        values[index]["index"] = json!(u64::MAX);
        assert!(parse(&encode(&values)).is_err());
    }
    let mut values = original.clone();
    values.insert(4, original[2].clone());
    assert!(parse(&encode(&values)).is_err());
    let mut values = original.clone();
    values.insert(3, original[1].clone());
    assert!(parse(&encode(&values)).is_err());
    for delta in [json!({}), json!({"type":"text_delta","text":42}),
        json!({"type":"input_json_delta","partial_json":"{}"})] {
        let mut values = original.clone();
        values[2]["delta"] = delta;
        assert!(parse(&encode(&values)).is_err());
    }
}

#[test]
fn multiple_blocks_keep_their_positions_and_opaque_fields() {
    let first = json!({"type":"thinking","thinking":"thought","signature":"signed"});
    let second = json!({"type":"redacted_thinking","data":"opaque"});
    let third = json!({"type":"text","text":"answer","citations":[{"url":"fixture"}]});
    let mut values = events(first.clone(), vec![], "end_turn");
    values.splice(3..3, [
        json!({"type":"content_block_start","index":1,"content_block":second}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"content_block_start","index":2,"content_block":third}),
        json!({"type":"content_block_stop","index":2}),
    ]);
    assert_eq!(parse(&encode(&values)).unwrap().content, vec![first, second, third]);
}

#[test]
fn multiline_crlf_comments_and_unknown_events_keep_valid_content() {
    let mut values = text_events("end_turn");
    values.insert(2, json!({"type":"future_event","value":"ignored"}));
    let stream: String = values.iter().map(|event| {
        let data: String = serde_json::to_string_pretty(event).unwrap().lines()
            .map(|line| format!("data:{line}\r\n")).collect();
        format!(": heartbeat\r\nevent: {}\r\n{data}\r\n", event["type"].as_str().unwrap())
    }).collect();
    assert_eq!(parse(stream.trim_end()).unwrap().content[0]["text"], "prefix suffix");
}
