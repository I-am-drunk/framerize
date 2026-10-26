use super::*;

#[test]
fn streamed_messages_reject_invalid_identity_metadata() {
    for (field, value) in [("type", json!("error")), ("type", json!(null)),
        ("role", json!("user")), ("role", json!(7)), ("id", json!("   "))] {
        let mut values = text_events("end_turn");
        values[0]["message"][field] = value;
        assert!(parse(&encode(&values)).is_err(), "accepted invalid {field}");
    }
}

#[test]
fn malformed_late_stop_metadata_cannot_hide_behind_an_earlier_reason() {
    for delta in [json!(null), json!([]), json!("delta"),
        json!({"stop_reason":" "}), json!({"stop_reason":7}),
        json!({"stop_sequence":[]}), json!({"stop_sequence":false})] {
        let mut values = text_events("end_turn");
        values.insert(5, json!({"type":"message_delta","delta":delta}));
        assert!(parse(&encode(&values)).is_err());
    }
}

#[test]
fn usage_only_updates_preserve_valid_message_and_stop_metadata() {
    let mut values = text_events("stop_sequence");
    values[0]["message"]["type"] = json!("message");
    values[0]["message"]["role"] = json!("assistant");
    values[4]["delta"]["stop_sequence"] = json!("END");
    values.insert(4, json!({"type":"message_delta","delta":{
        "stop_reason":null,"stop_sequence":null},"usage":{"output_tokens":5}}));
    values.insert(6, json!({"type":"message_delta","delta":{},
        "usage":{"output_tokens":10}}));
    let parsed = parse(&encode(&values)).unwrap();
    assert_eq!(parsed.id, "msg-fixture");
    assert_eq!(parsed.content[0]["text"], "prefix suffix");
    assert_eq!(parsed.stop_reason, "stop_sequence");
    assert_eq!(parsed.stop_sequence.as_deref(), Some("END"));
    assert_eq!(parsed.output_tokens, 10);
}
