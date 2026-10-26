use super::*;

fn message(id: &str, parts: Vec<Value>, status: &str) -> Value {
    json!({"id":id,"type":"message","role":"assistant","status":status,
        "content":parts,"phase":"final"})
}

fn plain(text: &str) -> Value {
    json!({"type":"output_text","text":text,"annotations":[]})
}

fn annotated() -> Value {
    json!({"type":"output_text","text":"word","annotations":[{
        "type":"url_citation","start_index":0,"end_index":4,
        "url":"https://example.invalid/fixture","title":"fixture"}],
        "logprobs":[{"token":"word","logprob":-0.5,"top_logprobs":[]}],
        "fixture_metadata":{"preserve":true}})
}

fn decode(frames: Vec<String>) -> Vec<Value> {
    frames.iter().map(|frame| serde_json::from_str(frame.lines()
        .find_map(|line| line.strip_prefix("data: ")).unwrap()).unwrap()).collect()
}

fn response(item: Value) -> AssembledResponses {
    AssembledResponses { id:"upstream".into(), created_at:1, output:vec![item],
        status:"completed".into(), incomplete_details:None, usage:json!({}),
        service_tier:Value::Null, input_tokens:1, output_tokens:2,
        chunks:2, metered_chunks:0 }
}

fn selected<'a>(events: &'a [Value], kind: &str) -> Vec<&'a Value> {
    events.iter().filter(|event| event["type"] == kind).collect()
}

fn assert_message(events: &[Value], expected: &Value) {
    let added = selected(events, "response.output_item.added");
    assert_eq!(added.len(), 1);
    let mut expected = expected.clone();
    expected["id"] = added[0]["item"]["id"].clone();
    for event in events.iter().filter(|event| event.get("content_index").is_some()) {
        assert_eq!(event["item_id"], expected["id"]);
        assert_eq!(event["output_index"], 0);
    }
    let parts = selected(events, "response.content_part.done");
    assert_eq!(parts.len(), expected["content"].as_array().unwrap().len());
    for (i, part) in parts.iter().enumerate() {
        assert_eq!(part["content_index"], i);
        assert_eq!(part["part"], expected["content"][i]);
    }
    assert_eq!(selected(events, "response.output_item.done")[0]["item"], expected);
    assert_eq!(selected(events, "response.completed")[0]["response"]["output"], json!([expected]));
}

#[test]
fn refusal_remains_a_refusal_in_every_completion_frame() {
    let item = message("upstream", vec![json!({"type":"refusal",
        "refusal":"I cannot complete this request."})], "completed");
    let mut composer = ResponsesComposer::new("fixture-model", "fixture-response");
    let mut events = decode(composer.push_items(&[item.clone()], false));
    events.extend(decode(composer.finish(&response(item.clone()))));
    assert_message(&events, &item);
    assert!(selected(&events, "response.output_text.delta").is_empty());
    assert!(selected(&events, "response.output_text.done").is_empty());
    assert_eq!(selected(&events, "response.refusal.delta")[0]["delta"], item["content"][0]["refusal"]);
    assert_eq!(selected(&events, "response.refusal.done")[0]["refusal"], item["content"][0]["refusal"]);
}

#[test]
fn multipart_content_keeps_annotations_logprobs_and_metadata() {
    let rich = annotated();
    let item = message("upstream", vec![rich.clone(),
        json!({"type":"refusal","refusal":"restricted portion"}), plain(" tail")], "completed");
    let mut composer = ResponsesComposer::new("fixture-model", "fixture-response");
    let mut events = decode(composer.push_items(&[item.clone()], false));
    events.extend(decode(composer.finish(&response(item.clone()))));
    assert_message(&events, &item);
    let annotations = selected(&events, "response.output_text.annotation.added");
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0]["annotation_index"], 0);
    assert_eq!(annotations[0]["content_index"], 0);
    assert_eq!(annotations[0]["annotation"], rich["annotations"][0]);
    assert_eq!(selected(&events, "response.output_text.delta")[0]["logprobs"], rich["logprobs"]);
    assert_eq!(selected(&events, "response.refusal.done")[0]["content_index"], 1);
}

#[test]
fn rich_text_parts_keep_their_offsets_across_continuation() {
    let mut citation = plain("word");
    citation["annotations"] = annotated()["annotations"].clone();
    let mut logprobs = plain("word");
    logprobs["logprobs"] = annotated()["logprobs"].clone();
    let mut opaque = plain("word");
    opaque["fixture_metadata"] = json!({"preserve":true});
    for rich in [citation, logprobs, opaque] {
        for parts in [vec![plain("prefix 🚀 "), rich.clone()], vec![rich, plain(" suffix")]] {
            let first = message("first", vec![parts[0].clone()], "incomplete");
            let last = message("last", vec![parts[1].clone()], "completed");
            let merged = message("first", parts.clone(), "completed");
            let mut composer = ResponsesComposer::new("fixture-model", "fixture-response");
            let mut events = decode(composer.push_items(&[first], true));
            events.extend(decode(composer.push_items(&[last], false)));
            events.extend(decode(composer.finish(&response(merged.clone()))));
            assert_message(&events, &merged);
        }
    }
}

#[test]
fn continuation_never_moves_text_across_a_refusal_part() {
    let parts = vec![plain("before"), json!({"type":"refusal","refusal":"middle"}), plain("after")];
    for split in [1, 2] {
        let first = message("first", parts[..split].to_vec(), "incomplete");
        let last = message("last", parts[split..].to_vec(), "completed");
        let merged = message("first", parts.clone(), "completed");
        let mut composer = ResponsesComposer::new("fixture-model", "fixture-response");
        let mut events = decode(composer.push_items(&[first], true));
        events.extend(decode(composer.push_items(&[last], false)));
        events.extend(decode(composer.finish(&response(merged.clone()))));
        assert_message(&events, &merged);
    }
}

#[test]
fn plain_continuation_joins_only_the_adjacent_boundary_parts() {
    let first = message("first", vec![plain("one"), plain("two")], "incomplete");
    let last = message("last", vec![plain("three"), plain("four")], "completed");
    let merged = message("first", vec![plain("one"), plain("twothree"), plain("four")], "completed");
    let mut composer = ResponsesComposer::new("fixture-model", "fixture-response");
    let mut events = decode(composer.push_items(&[first], true));
    events.extend(decode(composer.push_items(&[last], false)));
    events.extend(decode(composer.finish(&response(merged.clone()))));
    assert_message(&events, &merged);
    let added = selected(&events, "response.content_part.added");
    assert_eq!(added.len(), 3);
    let deltas = selected(&events, "response.output_text.delta");
    assert_eq!(deltas.iter().map(|event| event["content_index"].as_u64().unwrap())
        .collect::<Vec<_>>(), vec![0, 1, 1, 2]);
}
