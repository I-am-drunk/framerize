//! Inert response bytes only. No SDK, browser, credentials or HTTP transport.
use super::{output_facts, response_text};
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
fn response_decoding_accepts_one_initial_bom() {
    let json = serde_json::to_string(&response()).unwrap();
    let wire = format!("\u{feff}{json}");
    assert_eq!(response_text(wire.as_bytes()), Some(json.as_str()));
    assert_eq!(output_facts(wire.as_bytes(), RESPONSES, 200, true), (true, true));
    assert_eq!(output_facts(wire.as_bytes(), RESPONSES, 403, true), (true, false));
}

#[test]
fn response_decoding_rejects_double_or_noninitial_bom_before_json() {
    let json = serde_json::to_string(&response()).unwrap();
    for prefix in ["\u{feff}\u{feff}", " \u{feff}", "\n\u{feff}"] {
        let wire = format!("{prefix}{json}");
        assert_eq!(output_facts(wire.as_bytes(), RESPONSES, 200, true), (false, false));
    }
}

#[test]
fn response_decoding_preserves_bom_inside_json_text() {
    let mut value = response();
    value["output"][0]["content"][0]["text"] = json!("inside\u{feff}text");
    let json = serde_json::to_string(&value).unwrap();
    for prefix in ["", "\u{feff}"] {
        let wire = format!("{prefix}{json}");
        assert_eq!(response_text(wire.as_bytes()), Some(json.as_str()));
        assert_eq!(output_facts(wire.as_bytes(), RESPONSES, 200, true), (true, true));
    }
}

#[test]
fn response_decoding_rejects_invalid_utf8_after_bom() {
    for invalid in [b"\xff".as_slice(), b"\xe2\x82", b"\xed\xa0\x80"] {
        let mut wire = b"\xef\xbb\xbf{\"text\":\"".to_vec();
        wire.extend_from_slice(invalid);
        wire.extend_from_slice(b"\"}");
        assert_eq!(response_text(&wire), None);
        assert_eq!(output_facts(&wire, RESPONSES, 200, true), (false, false));
    }
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
