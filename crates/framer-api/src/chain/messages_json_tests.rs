use super::*;

fn message() -> Value {
    json!({"id":"fixture-message","type":"message","role":"assistant",
        "content":[{"type":"text","text":"fixture"}],"stop_reason":"end_turn",
        "stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":5}})
}

#[test]
fn non_message_envelopes_never_become_empty_success() {
    for body in [Value::Null, json!([]), json!(42), json!("text"), json!({}),
        json!({"type":"error","error":{"message":"fixture failure"}})] {
        assert!(decode(&body).is_err(), "accepted {body}");
    }
    let mut body = message();
    body["error"] = json!({"message":"fixture failure"});
    assert!(decode(&body).unwrap_err().starts_with("upstream 500 on /ai/v3/messages/:"));
    body.as_object_mut().unwrap().remove("error");
    body["type"] = json!("error");
    assert!(decode(&body).unwrap_err().starts_with("upstream 500 on /ai/v3/messages/:"));
}

#[test]
fn error_envelopes_keep_their_protocol_status_even_inside_http_200() {
    for (kind, status) in [("invalid_request_error", 400), ("permission_error", 403),
        ("rate_limit_error", 429), ("overloaded_error", 529)] {
        let body = json!({"type":"error","error":{"type":kind,"message":"fixture cause"}});
        let error = decode(&body).unwrap_err();
        let (actual, shaped) = framer_core::convert::error_json(
            framer_core::convert::ClientDialect::AnthropicMessages, &error);
        assert_eq!(actual, status);
        assert_eq!(shaped["error"]["type"], kind);
        assert_eq!(shaped["error"]["message"], "fixture cause");
    }
}

#[test]
fn invalid_completion_and_usage_fields_are_rejected() {
    for (field, value) in [
        ("content", json!(null)), ("content", json!({})),
        ("stop_reason", json!(null)), ("stop_reason", json!(" ")),
        ("stop_sequence", json!([])), ("usage", json!(null)),
        ("usage", json!(7)), ("usage", json!([])),
        ("usage", json!({"input_tokens":-1})),
        ("usage", json!({"output_tokens":"5"})),
        ("type", json!("event")), ("role", json!("user")),
        ("id", json!(42)),
    ] {
        let mut body = message();
        body[field] = value;
        assert!(decode(&body).is_err(), "accepted {body}");
    }
    for field in ["content", "stop_reason"] {
        let mut body = message();
        body.as_object_mut().unwrap().remove(field);
        assert!(decode(&body).is_err(), "accepted {body}");
    }
}

#[test]
fn malformed_blocks_and_complete_tool_inputs_are_rejected() {
    for block in [json!(null), json!(42), json!({}),
        json!({"type":"text","text":false}),
        json!({"type":"tool_use","id":"call","name":"fixture","input":"{}"}),
        json!({"type":"tool_use","id":"call","name":"fixture","input":[]}),
        json!({"type":"server_tool_use","id":"call","name":"fixture"}),
        json!({"type":"tool_use","id":"","name":"fixture","input":{}}),
        json!({"type":"tool_use","id":"call","name":null,"input":{}})] {
        let mut body = message();
        body["content"] = json!([block]);
        assert!(decode(&body).is_err(), "accepted {body}");
    }
}

#[test]
fn complete_content_and_usage_metadata_are_preserved() {
    let mut body = message();
    body["content"] = json!([
        {"type":"thinking","thinking":"fixture thought","signature":"signed-fixture"},
        {"type":"text","text":"fixture","citations":[{"fixture":true}]},
        {"type":"tool_use","id":"call","name":"fixture","input":{"nested":[1,true]}},
        {"type":"server_tool_use","id":"server","name":"search","input":{}},
        {"type":"future_result","opaque":{"fixture":[2,3]}}
    ]);
    body["usage"]["cache_read_input_tokens"] = json!(7);
    body["usage"]["server_tool_use"] = json!({"web_search_requests":1});
    body["stop_reason"] = json!("stop_sequence");
    body["stop_sequence"] = json!("fixture-end");
    let (blocks, stop, usage) = decode(&body).unwrap();
    assert_eq!(json!(blocks), body["content"]);
    assert_eq!(stop, "stop_sequence");
    assert_eq!(usage, body["usage"]);
}

#[test]
fn explicit_empty_completion_and_sparse_metadata_stay_valid() {
    let body = json!({"content":[],"stop_reason":"refusal"});
    let (blocks, stop, usage) = decode(&body).unwrap();
    assert!(blocks.is_empty());
    assert_eq!(stop, "refusal");
    assert_eq!(usage, json!({}));
}

#[test]
fn token_limited_tool_inputs_remain_available_to_the_retry_owner() {
    for kind in ["tool_use", "server_tool_use"] {
        let mut body = message();
        body["content"] = json!([{"type":kind,"id":"call","name":"fixture","input":null}]);
        body["stop_reason"] = json!("max_tokens");
        let (blocks, stop, _) = decode(&body).unwrap();
        assert_eq!(json!(blocks), body["content"]);
        assert_eq!(stop, "max_tokens");
    }
}
