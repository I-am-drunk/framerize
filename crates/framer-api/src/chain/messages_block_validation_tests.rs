use super::*;

#[test]
fn streamed_tools_require_nonempty_identity_fields() {
    for kind in ["tool_use", "server_tool_use"] {
        for field in ["id", "name"] {
            for value in [None, Some(Value::Null), Some(json!(42)), Some(json!("")), Some(json!("  "))] {
                let mut block = json!({"type":kind, "id":"call", "name":"lookup", "input":{}});
                if let Some(value) = value {
                    block[field] = value;
                } else {
                    block.as_object_mut().unwrap().remove(field);
                }
                assert!(parse(&encode(&events(block, vec![], "tool_use"))).is_err());
            }
        }
    }
}

#[test]
fn streamed_text_requires_string_text_even_without_deltas() {
    for value in [None, Some(Value::Null), Some(json!(42)), Some(json!({}))] {
        let mut block = json!({"type":"text"});
        if let Some(value) = value { block["text"] = value; }
        assert!(parse(&encode(&events(block, vec![], "end_turn"))).is_err());
    }
}

#[test]
fn streamed_blocks_reject_blank_types() {
    for kind in ["", "  "] {
        assert!(parse(&encode(&events(json!({"type":kind}), vec![], "end_turn"))).is_err());
    }
}

#[test]
fn streamed_blocks_preserve_empty_text_and_future_metadata() {
    for block in [json!({"type":"text", "text":"", "fixture_extra":true}),
        json!({"type":"fixture_future", "data":{"opaque":"kept"}})] {
        assert_eq!(parse(&encode(&events(block.clone(), vec![], "end_turn"))).unwrap().content,
            vec![block]);
    }
}
