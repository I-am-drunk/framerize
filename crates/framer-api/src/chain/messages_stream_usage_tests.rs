use super::*;

#[test]
fn streamed_usage_preserves_metadata_and_latest_counters() {
    let mut values = text_events("end_turn");
    values[0]["message"]["usage"] = json!({"input_tokens":17,"output_tokens":1,
        "cache_read_input_tokens":20,"cache_creation":{"ephemeral_5m_input_tokens":4},
        "service_tier":"standard"});
    values.insert(4, json!({"type":"message_delta","delta":{},"usage":{
        "output_tokens":5,"server_tool_use":{"web_search_requests":1}}}));
    values[5]["usage"] = json!({"input_tokens":19,"output_tokens":9,
        "server_tool_use":{"web_search_requests":2}});
    let parsed = parse(&encode(&values)).unwrap();
    assert_eq!((parsed.input_tokens, parsed.output_tokens), (19, 9));
    assert_eq!(parsed.usage, json!({"input_tokens":19,"output_tokens":9,
        "cache_read_input_tokens":20,"cache_creation":{"ephemeral_5m_input_tokens":4},
        "service_tier":"standard","server_tool_use":{"web_search_requests":2}}));
}

#[test]
fn streamed_usage_rejects_invalid_objects_and_counts() {
    for usage in [json!(null), json!([]), json!("usage"), json!(7),
        json!({"input_tokens":-1}), json!({"output_tokens":"9"}),
        json!({"output_tokens":null}), json!({"output_tokens":1.5}),
        json!({"output_tokens":18446744073709551616.0_f64})] {
        for start in [true, false] {
            let mut values = text_events("end_turn");
            if start { values[0]["message"]["usage"] = usage.clone(); }
            else { values[4]["usage"] = usage.clone(); }
            assert!(parse(&encode(&values)).unwrap_err().contains("usage metadata"));
        }
    }
    let mut values = text_events("end_turn");
    values[0]["message"]["usage"]["input_tokens"] = Value::Null;
    assert!(parse(&encode(&values)).unwrap_err().contains("usage metadata"));
}

#[test]
fn streamed_usage_supports_omitted_and_partial_snapshots() {
    for missing_start in [true, false] {
        let mut values = text_events("end_turn");
        if missing_start {
            values[0]["message"].as_object_mut().unwrap().remove("usage");
        } else {
            values[0]["message"]["usage"] = json!({"input_tokens":17,"output_tokens":3});
        }
        values[4].as_object_mut().unwrap().remove("usage");
        let parsed = parse(&encode(&values)).unwrap();
        let expected = if missing_start { (0, 0) } else { (17, 3) };
        assert_eq!((parsed.input_tokens, parsed.output_tokens), expected);
        assert_eq!(parsed.usage,
            json!({"input_tokens":expected.0,"output_tokens":expected.1}));
    }
}

#[test]
fn null_delta_usage_retains_prior_snapshot_fields() {
    let mut values = text_events("end_turn");
    let mut expected = json!({"input_tokens":17,"output_tokens":1,
        "cache_creation_input_tokens":5,"cache_read_input_tokens":4,
        "server_tool_use":{"web_search_requests":1},
        "output_tokens_details":{"thinking_tokens":3}});
    values[0]["message"]["usage"] = expected.clone();
    values[4]["usage"] = json!({"input_tokens":null,"output_tokens":9,
        "cache_creation_input_tokens":null,"cache_read_input_tokens":null,
        "server_tool_use":null,"output_tokens_details":null});
    expected["output_tokens"] = json!(9);
    let parsed = parse(&encode(&values)).unwrap();
    assert_eq!((parsed.input_tokens, parsed.output_tokens), (17, 9));
    assert_eq!(parsed.usage, expected);
}
