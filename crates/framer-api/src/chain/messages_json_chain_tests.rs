use super::*;

fn json_reply(body: Value) -> Reply {
    Reply { streaming:false, status:200, result:Ok(body.to_string()) }
}

fn valid_message(content: Vec<Value>, stop: &str) -> Value {
    json!({"id":"fixture-message","type":"message","role":"assistant",
        "content":content,"stop_reason":stop,"stop_sequence":null,
        "usage":{"input_tokens":19,"output_tokens":80}})
}

#[test]
fn error_and_missing_message_envelopes_never_publish_or_retry() {
    let mut failed = valid_message(vec![tool()], "tool_use");
    failed["error"] = json!({"message":"fixture upstream failure"});
    for body in [Value::Null, json!({}), failed,
        json!({"type":"error","error":{"message":"fixture failure"}})] {
        let observed = run(vec![json_reply(body)], 3);
        assert!(observed.result.is_err());
        assert!(observed.emitted.is_empty());
        assert_eq!(observed.requests.len(), 1);
        assert_eq!(observed.refunds, 0);
    }
}

#[test]
fn invalid_complete_tool_and_usage_data_never_reach_the_callback() {
    let mut bad_tool = tool();
    bad_tool["input"] = json!("{\"partial\":");
    let mut bad_usage = valid_message(vec![tool()], "tool_use");
    bad_usage["usage"] = json!(42);
    let mut missing_stop = valid_message(vec![tool()], "tool_use");
    missing_stop.as_object_mut().unwrap().remove("stop_reason");
    for body in [valid_message(vec![bad_tool], "tool_use"), bad_usage, missing_stop] {
        let observed = run(vec![json_reply(body)], 3);
        assert!(observed.result.is_err());
        assert!(observed.emitted.is_empty());
        assert_eq!(observed.requests.len(), 1);
        assert_eq!(observed.refunds, 0);
    }
}

#[test]
fn invalid_later_chunk_keeps_the_prefix_without_a_successful_completion() {
    let prefix = json!({"type":"text","text":"accepted prefix"});
    let observed = run(vec![
        json_reply(valid_message(vec![prefix.clone()], "max_tokens")),
        json_reply(json!({"content":[tool()],"error":{"message":"failed"}})),
    ], 3);
    assert!(observed.result.unwrap_err().contains("upstream error"));
    assert_eq!(observed.emitted, vec![prefix.clone()]);
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.requests[1]["request"]["messages"][1]["content"], json!([prefix]));
    assert_eq!(observed.refunds, 0);
}

#[test]
fn any_token_limited_tool_blocks_publication_at_the_retry_limit() {
    for kind in ["tool_use", "server_tool_use"] {
        for trailing_text in [false, true] {
            let mut call = tool();
            call["type"] = json!(kind);
            call["input"] = Value::Null;
            let mut blocks = vec![call];
            if trailing_text { blocks.push(json!({"type":"text","text":"trailing"})); }
            let observed = run(vec![json_reply(valid_message(blocks, "max_tokens"))], 1);
            assert!(observed.result.unwrap_err().contains("no remaining retry chunk"));
            assert!(observed.emitted.is_empty());
            assert_eq!(observed.requests.len(), 1);
            assert_eq!(observed.refunds, 0);
        }
    }
}

#[test]
fn complete_message_preserves_content_stop_sequence_and_usage() {
    let blocks = vec![
        json!({"type":"thinking","thinking":"fixture thought","signature":"fixture-signed"}),
        json!({"type":"text","text":"answer","citations":[{"fixture":true}]}), tool(),
    ];
    let mut body = valid_message(blocks.clone(), "stop_sequence");
    body["stop_sequence"] = json!("fixture-stop");
    body["usage"]["cache_read_input_tokens"] = json!(9);
    let observed = run(vec![json_reply(body.clone())], 3);
    let response = observed.result.unwrap();
    assert_eq!(response.id, "fixture-message");
    assert_eq!(response.content, blocks);
    assert_eq!(observed.emitted, blocks);
    assert_eq!(response.stop_reason, "stop_sequence");
    assert_eq!(response.stop_sequence.as_deref(), Some("fixture-stop"));
    assert_eq!(response.usage, body["usage"]);
    assert_eq!(observed.requests.len(), 1);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn partial_tool_input_is_retried_before_publishing_the_complete_tool() {
    let mut partial = tool();
    partial["input"] = Value::Null;
    let observed = run(vec![
        json_reply(valid_message(vec![partial], "max_tokens")),
        json_reply(valid_message(vec![tool()], "tool_use")),
    ], 2);
    let response = observed.result.unwrap();
    assert_eq!(response.content, vec![tool()]);
    assert_eq!(observed.emitted, vec![tool()]);
    assert_eq!(response.stop_reason, "tool_use");
    assert_eq!(response.output_tokens, 80);
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.refunds, 0);
}
