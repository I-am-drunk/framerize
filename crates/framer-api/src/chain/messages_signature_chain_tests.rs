use super::*;

#[test]
fn streaming_paths_preserve_complete_signature_values() {
    let initial = json!({"type":"thinking","thinking":"prefix","signature":"stale",
        "fixture_metadata":{"kept":true}});
    let raw = stream(initial, "end_turn");
    let mut values: Vec<Value> = raw.lines().filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).unwrap()).collect();
    values.splice(2..2, [
        json!({"type":"content_block_delta","index":0,
            "delta":{"type":"thinking_delta","thinking":" suffix"}}),
        json!({"type":"content_block_delta","index":0,
            "delta":{"type":"signature_delta","signature":"complete-signature"}}),
    ]);
    let raw: String = values.iter().map(|event| format!("data: {event}\n\n")).collect();
    let expected = json!({"type":"thinking","thinking":"prefix suffix",
        "signature":"complete-signature","fixture_metadata":{"kept":true}});
    for tool_repair in [false, true] {
        let mut replies = if tool_repair {
            vec![buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens")]
        } else { Vec::new() };
        replies.push(streamed(raw.clone()));
        let observed = if tool_repair { run(replies, 3) } else { run_opted_stream(replies, 1) };
        assert_eq!(observed.result.unwrap().content, vec![expected.clone()]);
        assert_eq!(observed.emitted, vec![expected.clone()]);
        let downstream = super::super::stream::parse(&observed.frames.join("\n\n")).unwrap();
        assert_eq!(downstream.content, vec![expected.clone()]);
        assert_eq!(observed.requests.len(), if tool_repair { 3 } else { 1 });
        assert_eq!(observed.refunds, 0);
    }
}
