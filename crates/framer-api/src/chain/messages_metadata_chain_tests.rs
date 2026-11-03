use super::*;

fn refusal_stream() -> (String, Value) {
    let details = json!({"type":"refusal","category":"reasoning_extraction",
        "explanation":"Provider explanation"});
    let events = [
        json!({"type":"message_start","message":{"id":"fixture-refusal"}}),
        json!({"type":"message_delta","delta":{"stop_reason":"refusal",
            "stop_details":details},"usage":{"output_tokens":1}}),
        json!({"type":"message_stop"}),
    ];
    (events.iter().map(|v| format!("data: {v}\n\n")).collect(), details)
}

#[test]
fn completed_refusal_in_either_fallback_preserves_details_and_ends_attempts() {
    for tool_repair in [false, true] {
        let (raw, details) = refusal_stream();
        let mut replies = if tool_repair {
            vec![buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens"),
                buffered(tool(), "max_tokens")]
        } else { vec![failed_request(), failed_request()] };
        replies.push(streamed(raw));
        let observed = run(replies, 5);
        let response = observed.result.unwrap();
        assert_eq!(response.stop_reason, "refusal");
        assert_eq!(response.stop_details, Some(details));
        assert!(observed.emitted.is_empty());
        assert_eq!(observed.requests.len(), if tool_repair { 4 } else { 3 });
        assert_eq!(observed.refunds, 0);
    }
}

#[test]
fn both_fallbacks_reject_late_malformed_stop_metadata() {
    for delta in [json!({"stop_reason":" "}), json!({"stop_sequence":false}), Value::Null] {
        let raw = stream(tool(), "tool_use");
        let mut values: Vec<Value> = raw.lines().filter_map(|line| line.strip_prefix("data: "))
            .map(|data| serde_json::from_str(data).unwrap()).collect();
        values.insert(4, json!({"type":"message_delta","delta":delta}));
        let raw: String = values.iter().map(|event| format!("data: {event}\n\n")).collect();
        for tool_repair in [false, true] {
            let mut replies = if tool_repair {
                vec![buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens"),
                    buffered(tool(), "max_tokens")]
            } else { vec![failed_request(), failed_request()] };
            replies.push(streamed(raw.clone()));
            let observed = run(replies, if tool_repair { 3 } else { 1 });
            assert!(observed.result.is_err());
            assert!(observed.emitted.is_empty());
            assert_eq!(observed.requests.len(), if tool_repair { 4 } else { 3 });
            assert_eq!(observed.refunds, 0);
        }
    }
}
