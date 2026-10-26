use super::*;

#[test]
fn citation_deltas_preserve_order_and_opaque_fields() {
    let first = json!({"type":"page_location", "cited_text":"an", "document_index":0,
        "start_page_number":1, "end_page_number":2, "fixture_extra":true});
    let second = json!({"type":"char_location", "cited_text":"swer", "document_index":0,
        "start_char_index":0, "end_char_index":4});
    for initial in [None, Some(Value::Null), Some(json!([])), Some(json!([first.clone()]))] {
        let mut expected = initial.as_ref().and_then(Value::as_array).cloned().unwrap_or_default();
        expected.extend([first.clone(), second.clone()]);
        let mut block = json!({"type":"text", "text":""});
        if let Some(initial) = initial { block["citations"] = initial; }
        let values = events(block, vec![
            json!({"type":"text_delta", "text":"an"}),
            json!({"type":"citations_delta", "citation":first}),
            json!({"type":"text_delta", "text":"swer"}),
            json!({"type":"citations_delta", "citation":second}),
        ], "end_turn");
        let response = parse(&encode(&values)).unwrap();
        assert_eq!(response.content, vec![json!({"type":"text", "text":"answer", "citations":expected})]);
    }
}

#[test]
fn malformed_citation_deltas_are_rejected() {
    let text = json!({"type":"text", "text":"answer"});
    let mut cases: Vec<_> = [Value::Null, json!("bad"), json!(42), json!([])]
        .into_iter().map(|citation| (text.clone(), citation)).collect();
    cases.push((json!({"type":"tool_use", "id":"tool", "name":"lookup", "input":{}}),
        json!({"type":"page_location"})));
    cases.push((json!({"type":"text", "text":"answer", "citations":42}),
        json!({"type":"page_location"})));
    for (block, citation) in cases {
        let values = events(block, vec![json!({"type":"citations_delta", "citation":citation})], "end_turn");
        assert!(parse(&encode(&values)).is_err());
    }
}
