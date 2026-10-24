//! Inert response bytes only. No SDK, browser, credentials or HTTP transport.
use super::output_facts;
use serde_json::{json, Value};

const RESPONSES: &str = "/ai/v3/responses/";
const MESSAGES: &str = "/ai/v3/messages/";

fn facts(value: &Value, path: &str) -> (bool, bool) {
    output_facts(&serde_json::to_vec(value).unwrap(), path, 200, true)
}

#[test]
fn responses_report_completed_nonempty_model_text() {
    assert_eq!(facts(&response(), RESPONSES), (true, true));
    assert_eq!(facts(&json!({"ok":true}), RESPONSES), (true, false));
}

#[test]
fn responses_keep_incomplete_error_and_invalid_shapes_distinct() {
    for (key, invalid) in [("id", json!("")), ("id", json!(17)),
        ("status", json!("incomplete")), ("incomplete_details", json!({})),
        ("error", json!({"message":"private-error-marker"})),
        ("output", json!([])), ("output", json!({"text":"private-text"}))] {
        let mut value = response();
        value[key] = invalid;
        assert_eq!(facts(&value, RESPONSES), (true, false), "{key}");
    }
}

#[test]
fn responses_require_text_inside_a_message() {
    for invalid in [json!(" \n\t\u{a0}"), json!(false), Value::Null] {
        let mut value = response();
        value["output"][0]["content"][0]["text"] = invalid;
        assert_eq!(facts(&value, RESPONSES), (true, false));
    }
    let mut value = response();
    value["output"][0]["type"] = json!("function_call");
    assert_eq!(facts(&value, RESPONSES), (true, false));
}

#[test]
fn messages_report_text_for_each_accepted_stop_reason() {
    let mut value = json!({"id":"fixture", "content":[{"type":"text", "text":"ok"}]});
    for reason in ["end_turn", "stop_sequence", "tool_use"] {
        value["stop_reason"] = json!(reason);
        assert_eq!(facts(&value, MESSAGES), (true, true));
    }
    value["stop_reason"] = json!("max_tokens");
    assert_eq!(facts(&value, MESSAGES), (true, false));
}

#[test]
fn messages_do_not_count_tool_only_or_whitespace_output_as_text() {
    for content in [json!([]), json!([{"type":"tool_use", "text":"private"}]),
        json!([{"type":"text", "text":"\n\t "}]), json!([{"type":"text", "text":1}])] {
        let value = json!({"id":"fixture", "stop_reason":"end_turn", "content":content});
        assert_eq!(facts(&value, MESSAGES), (true, false));
    }
}

#[test]
fn output_evidence_requires_http_200_json_and_known_route() {
    let bytes = serde_json::to_vec(&response()).unwrap();
    for status in [201, 403, 429, 500] {
        assert_eq!(output_facts(&bytes, RESPONSES, status, true), (true, false));
    }
    assert_eq!(output_facts(&bytes, RESPONSES, 200, false), (false, false));
    assert_eq!(output_facts(&bytes, "/ai/v3/chat/", 200, true), (true, false));
}

#[test]
fn malformed_or_nonobject_json_has_no_model_output_evidence() {
    for bytes in [b"".as_slice(), b"{", b"[]", b"null", b"true", b"{}{}",
        b"{\"id\":\"fixture\",\"status\":\"completed\",\"output\":["] {
        assert_eq!(output_facts(bytes, RESPONSES, 200, true), (false, false));
    }
}

fn response() -> Value {
    json!({"id":"fixture-response", "status":"completed", "output":[{
        "type":"message", "content":[{"type":"output_text", "text":"fixture text"}]
    }]})
}
