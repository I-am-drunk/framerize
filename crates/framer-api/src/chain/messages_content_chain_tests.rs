use super::*;

fn cited(text: &str, page: u32) -> Value {
    json!({"type":"text", "text":text, "citations":[{
        "type":"page_location", "document_index":0, "document_title":"fixture",
        "start_page_number":page, "end_page_number":page + 1, "cited_text":text,
    }]})
}

fn buffered_blocks(content: Vec<Value>, stop: &str) -> Reply {
    Reply { streaming:false, status:200, result:Ok(json!({
        "id":"fixture-content", "content":content, "stop_reason":stop,
        "usage":{"input_tokens":19,"output_tokens":80},
    }).to_string()) }
}

fn client_reply(observed: &Observed) -> StreamedChunk {
    super::super::stream::parse(&(observed.frames.join("\n\n") + "\n\n")).unwrap()
}

fn cited_stream(citation: Value) -> String {
    let raw = stream(json!({"type":"text", "text":"answer"}), "end_turn");
    let mut values: Vec<Value> = raw.lines().filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).unwrap()).collect();
    values.insert(2, json!({"type":"content_block_delta", "index":0,
        "delta":{"type":"citations_delta", "citation":citation}}));
    values.iter().map(|value| format!("data: {value}\n\n")).collect()
}

#[test]
fn buffered_citations_match_client_stream_and_continuation_history() {
    let first = cited("first", 1);
    let second = cited("second", 2);
    let head = vec![json!({"type":"text","text":"prefix"}), first];
    let mut call = tool();
    call["caller"] = json!({"type":"direct"});
    let tail = vec![second.clone(), json!({"type":"text","text":"suf"}),
        json!({"type":"text","text":"fix"}), call.clone()];
    let observed = run(vec![buffered_blocks(head.clone(), "max_tokens"),
        buffered_blocks(tail, "tool_use")], 2);
    let response = observed.result.as_ref().unwrap();
    let mut expected = head.clone();
    expected.extend([second, json!({"type":"text","text":"suffix"}), call]);
    assert_eq!(response.content, expected);
    let client = client_reply(&observed);
    assert_eq!(client.content, expected);
    assert_eq!(client.stop_reason, "tool_use");
    assert_eq!(response.output_tokens, 160);
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.requests[1]["request"]["messages"][1]["content"], json!(head));
    assert_eq!(observed.refunds, 0);
}

#[test]
fn metered_citations_reach_assembly_and_client_stream() {
    let expected = cited("answer", 1);
    let observed = run(vec![failed_request(), failed_request(),
        streamed(cited_stream(expected["citations"][0].clone()))], 1);
    let response = observed.result.as_ref().unwrap();
    assert_eq!(response.content, vec![expected.clone()]);
    assert_eq!(observed.emitted, vec![expected.clone()]);
    assert_eq!(client_reply(&observed).content, vec![expected]);
    assert_eq!((response.chunks, response.metered_chunks), (1, 1));
    assert_eq!(observed.requests.len(), 3);
    assert_eq!(observed.refunds, 1);
}

#[test]
fn malformed_metered_citations_never_publish_content() {
    let observed = run(vec![failed_request(), failed_request(),
        streamed(cited_stream(Value::Null))], 1);
    assert!(observed.result.as_ref().unwrap_err().contains("citation"));
    assert!(observed.emitted.is_empty());
    assert!(!observed.frames.iter().any(|frame| frame.contains("content_block_start")));
    assert_eq!(observed.requests.len(), 3);
    assert_eq!(observed.refunds, 1);
}
