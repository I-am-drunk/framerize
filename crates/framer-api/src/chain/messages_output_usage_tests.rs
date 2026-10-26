use super::*;

#[test]
fn continued_messages_stream_preserves_aggregate_usage() {
    let first = buffered(json!({"type":"text","text":"prefix "}), "max_tokens");
    let mut last = buffered(json!({"type":"text","text":"suffix"}), "stop_sequence");
    let mut body: Value = serde_json::from_str(last.result.as_ref().unwrap()).unwrap();
    body["stop_sequence"] = json!("END");
    body["usage"] = json!({"input_tokens":37,"output_tokens":40,
        "cache_creation_input_tokens":5,"cache_read_input_tokens":7,
        "server_tool_use":{"web_search_requests":2},
        "output_tokens_details":{"thinking_tokens":20},"service_tier":"standard"});
    last.result = Ok(body.to_string());
    let observed = run(vec![first, last], 2);
    let response = observed.result.unwrap();
    let parsed = super::super::stream::parse(&observed.frames.join("\n\n")).unwrap();
    assert_eq!((parsed.input_tokens, parsed.output_tokens), (37, 120));
    assert_eq!(parsed.content, response.content);
    assert_eq!(parsed.usage, json!({"input_tokens":37,"output_tokens":120,
        "cache_creation_input_tokens":5,"cache_read_input_tokens":7,
        "server_tool_use":{"web_search_requests":2},
        "output_tokens_details":{"thinking_tokens":20}}));
    assert_eq!(parsed.stop_reason, "stop_sequence");
    assert_eq!(parsed.stop_sequence.as_deref(), Some("END"));
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.refunds, 0);
}
