use super::*;

fn message(text: &str) -> Value {
    json!({"type":"message","role":"assistant","status":"completed",
        "content":[{"type":"output_text","text":text}]})
}

fn render(chunks: Vec<(Vec<Value>, bool)>, output: Vec<Value>) -> Vec<Value> {
    let mut composer = ResponsesComposer::new("fixture-model", "fixture-response");
    let mut frames = composer.open();
    for (items, continues) in chunks {
        frames.extend(composer.push_items(&items, continues));
    }
    frames.extend(composer.finish(&AssembledResponses {
        id:"upstream".into(), created_at:1, output, status:"completed".into(),
        incomplete_details:None, usage:json!({}), service_tier:Value::Null,
        input_tokens:0, output_tokens:0, chunks:1, metered_chunks:0,
    }));
    frames.iter().map(|frame| serde_json::from_str(frame.lines()
        .find_map(|line| line.strip_prefix("data: ")).unwrap()).unwrap()).collect()
}

fn assert_items(events: &[Value], expected: &[Value]) {
    let output = events.last().unwrap()["response"]["output"].as_array().unwrap();
    let added: Vec<_> = events.iter().filter(|e| e["type"] == "response.output_item.added").collect();
    let done: Vec<_> = events.iter().filter(|e| e["type"] == "response.output_item.done").collect();
    assert_eq!((added.len(), done.len(), output.len()), (expected.len(), expected.len(), expected.len()));
    for (index, source) in expected.iter().enumerate() {
        assert_eq!(added[index]["output_index"], index);
        assert_eq!(done[index]["output_index"], index);
        assert_eq!(added[index]["item"]["id"], output[index]["id"]);
        assert_eq!(done[index]["item"], output[index]);
        let mut item = source.clone();
        if item["type"] == "message" {
            item["id"] = output[index]["id"].clone();
        }
        assert_eq!(output[index], item);
    }
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["sequence_number"], index);
        if let Some(id) = event.get("item_id") {
            assert_eq!(*id, output[event["output_index"].as_u64().unwrap() as usize]["id"]);
        }
    }
}

#[test]
fn builtin_output_separates_messages_and_preserves_later_tool_indices() {
    let search = json!({"type":"web_search_call","id":"search-id","status":"completed",
        "action":{"type":"search","query":"fixture"},"extension":{"keep":true}});
    let tool = json!({"type":"function_call","id":"tool-id","call_id":"call-id",
        "name":"read","namespace":"files","arguments":"{}","status":"completed"});
    let output = vec![message("before"), search.clone(), message("after"), tool];
    let events = render(vec![(output.clone(), false)], output.clone());
    assert_items(&events, &output);
    let search_added = events.iter().find(|e|
        e["type"] == "response.output_item.added" && e["output_index"] == 1).unwrap();
    assert_eq!(search_added["item"], search);
    let arguments = events.iter().find(|e|
        e["type"] == "response.function_call_arguments.done").unwrap();
    assert_eq!(arguments["output_index"], 3);
    assert_eq!(arguments["item_id"], "tool-id");
    let text: Vec<_> = events.iter().filter(|e| e["type"] == "response.output_text.done")
        .map(|e| e["text"].as_str().unwrap()).collect();
    assert_eq!(text, vec!["before", "after"]);
}

#[test]
fn future_output_remains_verbatim_between_chunks_with_optional_ids() {
    for item in [json!({"type":"future_output","role":"user","status":false,
        "extension":{"keep":[1,2]}}),
        json!({"type":"future_output","id":null,"opaque":["retained"]})] {
        let first = message("first");
        let last = message("last");
        let output = vec![first.clone(), item.clone(), last.clone()];
        let events = render(vec![(vec![first], true), (vec![item.clone()], true),
            (vec![last], false)], output.clone());
        assert_items(&events, &output);
        for kind in ["response.output_item.added", "response.output_item.done"] {
            let event = events.iter().find(|e| e["type"] == kind && e["output_index"] == 1).unwrap();
            assert_eq!(event["item"], item);
        }
    }
}
