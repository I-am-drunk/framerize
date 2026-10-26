use super::*;

fn message(id: &str, text: &str, status: &str) -> Value {
    json!({"id":id,"type":"message","role":"assistant","status":status,
        "content":[{"type":"output_text","text":text,"annotations":[]}]})
}

fn result(output: Vec<Value>) -> AssembledResponses {
    AssembledResponses { id:"upstream-response".into(), created_at:123,
        output, status:"completed".into(), incomplete_details:None,
        usage:json!({"input_tokens":7,"output_tokens":11,"total_tokens":18}),
        service_tier:json!("priority"), input_tokens:7, output_tokens:11,
        chunks:2, metered_chunks:0 }
}

fn decode(frames: Vec<String>) -> Vec<Value> {
    frames.iter().map(|frame| {
        let data = frame.lines().find_map(|line| line.strip_prefix("data: ")).unwrap();
        serde_json::from_str(data).unwrap()
    }).collect()
}

fn selected<'a>(events: &'a [Value], kind: &str) -> Vec<&'a Value> {
    events.iter().filter(|event| event["type"] == kind).collect()
}

fn assert_matching_items(events: &[Value]) -> Value {
    let added = selected(events, "response.output_item.added");
    let done = selected(events, "response.output_item.done");
    let finals = selected(events, "response.completed");
    assert_eq!(finals.len(), 1);
    let full = finals[0]["response"].clone();
    let output = full["output"].as_array().unwrap();
    assert_eq!(added.len(), output.len());
    assert_eq!(done.len(), output.len());
    for (i, item) in output.iter().enumerate() {
        assert_eq!(added[i]["output_index"], i);
        assert_eq!(done[i]["output_index"], i);
        assert_eq!(added[i]["item"]["id"], item["id"]);
        assert_eq!(done[i]["item"], *item);
    }
    full
}

#[test]
fn completed_message_keeps_its_streamed_identity() {
    let output = vec![message("upstream-item", "fixture text", "completed")];
    let mut composer = ResponsesComposer::new("fixture-model", "local-response");
    let mut events = decode(composer.push_items(&output, false));
    assert_eq!(selected(&events, "response.output_item.done").len(), 1);
    events.extend(decode(composer.finish(&result(output))));
    let full = assert_matching_items(&events);
    assert_eq!(full["id"], "resp_local-response");
    assert_eq!(full["usage"]["total_tokens"], 18);
    assert_eq!(full["service_tier"], "priority");
}

#[test]
fn missing_message_id_is_repaired_consistently() {
    let mut item = message("", "fixture text", "completed");
    item.as_object_mut().unwrap().remove("id");
    let mut composer = ResponsesComposer::new("fixture-model", "local-response");
    let mut events = decode(composer.push_items(&[item.clone()], false));
    events.extend(decode(composer.finish(&result(vec![item]))));
    let full = assert_matching_items(&events);
    assert!(full["output"][0]["id"].as_str().unwrap().starts_with("msg_"));
}

#[test]
fn final_chunk_continues_the_open_message_before_closing() {
    let mut composer = ResponsesComposer::new("fixture-model", "local-response");
    let mut events = decode(composer.push_items(
        &[message("first", "prefix ", "incomplete")], true));
    assert!(selected(&events, "response.output_item.done").is_empty());
    events.extend(decode(composer.push_items(
        &[message("last", "suffix", "completed")], false)));
    assert_eq!(selected(&events, "response.output_item.added").len(), 1);
    assert_eq!(selected(&events, "response.output_item.done").len(), 1);
    let deltas: String = selected(&events, "response.output_text.delta").iter()
        .map(|event| event["delta"].as_str().unwrap()).collect();
    assert_eq!(deltas, "prefix suffix");
    events.extend(decode(composer.finish(&result(vec![
        message("first", "prefix suffix", "completed")]))));
    assert_matching_items(&events);
}

#[test]
fn adjacent_messages_match_the_chains_merged_output() {
    let mut composer = ResponsesComposer::new("fixture-model", "local-response");
    let mut events = decode(composer.push_items(&[
        message("first", "one ", "completed"),
        message("second", "two", "completed")], false));
    events.extend(decode(composer.finish(&result(vec![
        message("first", "one two", "completed")]))));
    let full = assert_matching_items(&events);
    assert_eq!(full["output"].as_array().unwrap().len(), 1);
    assert_eq!(selected(&events, "response.output_text.done")[0]["text"], "one two");
}

#[test]
fn final_token_limited_message_retains_incomplete_status() {
    let output = vec![message("first", "partial", "incomplete")];
    let mut composer = ResponsesComposer::new("fixture-model", "local-response");
    let mut events = decode(composer.push_items(&output, true));
    let mut response = result(output);
    response.status = "incomplete".into();
    response.incomplete_details = Some(json!({"reason":"max_output_tokens"}));
    events.extend(decode(composer.finish(&response)));
    let full = assert_matching_items(&events);
    assert_eq!(full["status"], "incomplete");
    assert_eq!(full["incomplete_details"]["reason"], "max_output_tokens");
}

#[test]
fn tools_and_reasoning_keep_identity_between_messages() {
    let reasoning = json!({"type":"reasoning","id":"reasoning-fixture",
        "encrypted_content":"synthetic","summary":[]});
    let call = json!({"type":"function_call","id":"function-fixture",
        "call_id":"call-function","name":"read","namespace":"files",
        "status":"completed","arguments":"{\"path\":\"fixture\"}"});
    let custom = json!({"type":"custom_tool_call","id":"custom-fixture",
        "call_id":"call-custom","name":"exec","namespace":"tools",
        "input":"fixture();"});
    let output = vec![message("first", "before", "completed"), reasoning.clone(),
        call.clone(), custom.clone(), message("last", "after", "completed")];
    let mut composer = ResponsesComposer::new("fixture-model", "local-response");
    let mut events = decode(composer.push_items(&output, false));
    events.extend(decode(composer.finish(&result(output))));
    let full = assert_matching_items(&events);
    assert_eq!(full["output"][1], reasoning);
    assert_eq!(full["output"][2], call);
    assert_eq!(full["output"][3], custom);
    assert_ne!(full["output"][0]["id"], full["output"][4]["id"]);
}
