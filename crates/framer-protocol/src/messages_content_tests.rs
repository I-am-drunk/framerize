use super::*;

fn cited(text: &str, page: u32) -> Value {
    json!({"type":"text", "text":text, "citations":[{
        "type":"page_location", "document_index":0, "document_title":"fixture",
        "start_page_number":page, "end_page_number":page + 1, "cited_text":text,
    }], "fixture_metadata":{"kept":true}})
}

fn tool() -> Value {
    json!({"type":"tool_use", "id":"call-fixture", "name":"lookup",
        "input":{"value":42}, "caller":{"type":"direct"}})
}

fn assembled(content: Vec<Value>) -> AssembledResponse {
    AssembledResponse { id:"msg-fixture".into(), content, stop_reason:"tool_use".into(),
        stop_sequence:None, usage:json!({"input_tokens":2,"output_tokens":3}),
        input_tokens:2, output_tokens:3, chunks:1, metered_chunks:0 }
}

fn events(frames: Vec<String>) -> Vec<Value> {
    frames.iter().map(|frame| serde_json::from_str(frame.lines()
        .find_map(|line| line.strip_prefix("data: ")).unwrap()).unwrap()).collect()
}

fn deltas(events: &[Value], kind: &str, field: &str) -> Vec<(u64, Value)> {
    events.iter().filter(|event| event["type"] == "content_block_delta"
        && event["delta"]["type"] == kind)
        .map(|event| (event["index"].as_u64().unwrap(), event["delta"][field].clone())).collect()
}

#[test]
fn message_merge_keeps_cited_boundaries_and_joins_only_plain_text() {
    let first = cited("first", 1);
    let second = cited("second", 3);
    let mut blocks = vec![json!({"type":"text","text":"a"})];
    merge_blocks_local(&mut blocks, vec![json!({"type":"text","text":"b"}),
        first.clone(), second.clone(), json!({"type":"text","text":"c"})]);
    merge_blocks_local(&mut blocks, vec![json!({"type":"text","text":"d"}), tool()]);
    assert_eq!(blocks, vec![json!({"type":"text","text":"ab"}), first, second,
        json!({"type":"text","text":"cd"}), tool()]);
}

#[test]
fn message_snapshot_stream_preserves_citations_and_tool_metadata() {
    let text = cited("answer", 2);
    let response = assembled(vec![text.clone(), tool()]);
    let values = events(render_sse(ClientDialect::AnthropicMessages, &response, "fixture", false));
    let starts: Vec<_> = values.iter().filter(|event| event["type"] == "content_block_start")
        .map(|event| event["content_block"].clone()).collect();
    let mut empty_text = text.clone();
    empty_text["text"] = json!("");
    empty_text["citations"] = json!([]);
    let mut empty_tool = tool();
    empty_tool["input"] = json!({});
    assert_eq!(starts, vec![empty_text, empty_tool]);
    assert_eq!(deltas(&values, "citations_delta", "citation"), vec![(0, text["citations"][0].clone())]);
    assert_eq!(deltas(&values, "input_json_delta", "partial_json"), vec![(1, json!(tool()["input"].to_string()))]);
}

#[test]
fn message_composer_keeps_rich_blocks_between_plain_continuations() {
    let first = cited("first", 1);
    let second = cited("second", 3);
    let mut composer = SseComposer::new(ClientDialect::AnthropicMessages, "msg-fixture", "fixture");
    let mut frames = composer.push_chunk(&[json!({"type":"text","text":"a"})]);
    frames.extend(composer.push_chunk(&[json!({"type":"text","text":"b"}), first.clone()]));
    frames.extend(composer.push_chunk(&[second.clone(), json!({"type":"text","text":"c"}), tool()]));
    frames.extend(composer.finish(&assembled(vec![]), "fixture", false));
    let values = events(frames);
    let starts: Vec<_> = values.iter().filter(|event| event["type"] == "content_block_start").collect();
    assert_eq!(starts.len(), 5);
    for (index, event) in starts.iter().enumerate() {
        assert_eq!(event["index"], index);
    }
    assert_eq!(starts[1]["content_block"]["fixture_metadata"], first["fixture_metadata"]);
    assert_eq!(starts[2]["content_block"]["fixture_metadata"], second["fixture_metadata"]);
    assert_eq!(starts[4]["content_block"]["caller"], tool()["caller"]);
    assert_eq!(deltas(&values, "text_delta", "text"), vec![
        (0, json!("a")), (0, json!("b")), (1, json!("first")),
        (2, json!("second")), (3, json!("c")),
    ]);
    assert_eq!(deltas(&values, "citations_delta", "citation"), vec![
        (1, first["citations"][0].clone()), (2, second["citations"][0].clone()),
    ]);
    let stopped: Vec<_> = values.iter().filter(|event| event["type"] == "content_block_stop")
        .map(|event| event["index"].as_u64().unwrap()).collect();
    assert_eq!(stopped, vec![0, 1, 2, 3, 4]);
    assert_eq!(values.last().unwrap()["type"], "message_stop");
}
