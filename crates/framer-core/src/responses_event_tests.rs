use super::*;

fn tools() -> Vec<Value> {
    vec![
        json!({"type":"function_call","id":"fc-upstream","call_id":"call/function",
            "name":"read","namespace":"files","arguments":"{\"path\":\"α\"}",
            "status":"completed","extension":{"preserve":[1,2]}}),
        json!({"type":"custom_tool_call","id":"ctc-upstream","call_id":"call/custom",
            "name":"run","namespace":"tools","input":"echo 'α'\n",
            "status":"completed","extension":{"preserve":[3,4]}}),
    ]
}

fn result(output: Vec<Value>) -> AssembledResponses {
    AssembledResponses { id:"upstream-response".into(), created_at:123,
        output, status:"completed".into(), incomplete_details:None,
        usage:json!({"input_tokens":7,"output_tokens":11,"total_tokens":18}),
        service_tier:json!("priority"), input_tokens:7, output_tokens:11,
        chunks:2, metered_chunks:0 }
}

fn message(text: &str, status: &str) -> Value {
    json!({"type":"message","id":"upstream-message","role":"assistant",
        "status":status,"content":[{"type":"output_text","text":text}]})
}

fn decode(frames: Vec<String>) -> Vec<Value> {
    frames.iter().filter_map(|frame| {
        let data = frame.lines().find_map(|line| line.strip_prefix("data: "))?;
        let event: Value = serde_json::from_str(data).unwrap();
        let kind = frame.lines().find_map(|line| line.strip_prefix("event: ")).unwrap();
        assert_eq!(event["type"], kind);
        Some(event)
    }).collect()
}

fn assert_sequence(events: &[Value]) {
    assert!(!events.is_empty());
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["sequence_number"].as_u64(), Some(index as u64), "{event}");
    }
}

fn item_event<'a>(events: &'a [Value], kind: &str, index: usize) -> &'a Value {
    let selected: Vec<_> = events.iter().filter(|event|
        event["type"] == kind && event["output_index"] == index).collect();
    assert_eq!(selected.len(), 1, "{kind} at {index}");
    selected[0]
}

fn assert_tool(events: &[Value], index: usize, source: &Value) -> Value {
    let terminal = events.iter().find(|event| event["type"] == "response.completed").unwrap();
    let item = terminal["response"]["output"][index].clone();
    assert!(item["id"].is_string());
    let mut expected = source.clone();
    expected["id"] = item["id"].clone();
    assert_eq!(item, expected);
    assert_eq!(item_event(events, "response.output_item.done", index)["item"], item);
    let (field, stem) = if source["type"] == "function_call" {
        expected["status"] = json!("in_progress");
        ("arguments", "response.function_call_arguments")
    } else { ("input", "response.custom_tool_call_input") };
    expected[field] = json!("");
    assert_eq!(item_event(events, "response.output_item.added", index)["item"], expected);
    let delta = item_event(events, &format!("{stem}.delta"), index);
    let done = item_event(events, &format!("{stem}.done"), index);
    assert_eq!(delta["item_id"], item["id"]);
    assert_eq!(done["item_id"], item["id"]);
    assert_eq!(delta["delta"], item[field]);
    assert_eq!(done[field], item[field]);
    assert_eq!(done["call_id"], source["call_id"]);
    assert_eq!(done["name"], source["name"]);
    item
}

#[test]
fn supplied_tool_ids_link_argument_events_to_final_items() {
    let output = tools();
    let mut composer = ResponsesComposer::new("fixture-model", "fixture-response");
    let mut frames = composer.open();
    frames.extend(composer.push_items(&output, false));
    frames.extend(composer.finish(&result(output.clone())));
    let events = decode(frames);
    for (index, source) in output.iter().enumerate() {
        assert_eq!(assert_tool(&events, index, source), *source);
    }
    assert_sequence(&events);
}

#[test]
fn anonymous_tool_ids_and_optional_status_survive_finalization() {
    for null_id in [false, true] {
        let mut output = tools();
        for item in &mut output {
            item.as_object_mut().unwrap().remove("status");
            item.as_object_mut().unwrap().remove("id");
            if null_id { item["id"] = Value::Null; }
        }
        let mut composer = ResponsesComposer::new("fixture-model", "fixture-response");
        let mut frames = composer.open();
        for item in &output {
            frames.extend(composer.push_items(&[item.clone()], false));
        }
        frames.extend(composer.finish(&result(output.clone())));
        let events = decode(frames);
        let first = assert_tool(&events, 0, &output[0]);
        let second = assert_tool(&events, 1, &output[1]);
        assert!(first["id"].as_str().unwrap().starts_with("fc_"));
        assert!(second["id"].as_str().unwrap().starts_with("ctc_"));
        assert_ne!(first["id"], second["id"]);
        assert!(first.get("status").is_none());
        assert!(second.get("status").is_none());
        assert_sequence(&events);
    }
}

#[test]
fn one_sequence_covers_prelude_rich_continuation_tools_and_completion() {
    let first = message("prefix ", "incomplete");
    let mut last = message("suffix", "completed");
    last["content"] = json!([
        {"type":"output_text","text":"suffix","annotations":[{"fixture":true}]},
        {"type":"refusal","refusal":"no"},{"type":"future_part","opaque":true}]);
    let mut merged = first.clone();
    merge_response_message(&mut merged, &last);
    let mut second = vec![last, json!({"type":"reasoning","id":"reasoning-id","summary":[]})];
    second.extend(tools());
    let mut output = vec![merged];
    output.extend(second[1..].iter().cloned());
    let mut composer = ResponsesComposer::new("fixture-model", "fixture-response");
    let mut frames = composer.open();
    frames.extend(composer.push_items(&[first], true));
    frames.push(ping_frame(ClientDialect::OpenAiResponses));
    frames.extend(composer.push_items(&second, false));
    frames.extend(composer.finish(&result(output)));
    let events = decode(frames);
    assert_sequence(&events);
    assert_eq!(events[0]["type"], "response.created");
    assert_eq!(events[1]["type"], "response.in_progress");
    assert_eq!(events.last().unwrap()["type"], "response.completed");
    for kind in ["response.output_text.annotation.added", "response.refusal.delta",
        "response.content_part.done", "response.output_text.done"] {
        assert!(events.iter().any(|event| event["type"] == kind), "{kind}");
    }
    assert_tool(&events, 2, &second[2]);
    assert_tool(&events, 3, &second[3]);
}

#[test]
fn failure_after_output_keeps_sequence_without_finishing_pending_content() {
    let mut composer = ResponsesComposer::new("fixture-model", "fixture-response");
    let mut frames = composer.open();
    frames.extend(composer.push_items(&[message("partial", "incomplete")], true));
    let tail = composer.fail("Cleanup");
    assert_eq!(tail.len(), 1);
    frames.extend(tail);
    let events = decode(frames);
    assert_sequence(&events);
    assert!(events.iter().any(|event| event["type"] == "response.output_text.delta"));
    for kind in ["response.output_text.done", "response.content_part.done",
        "response.output_item.done", "response.completed"] {
        assert!(!events.iter().any(|event| event["type"] == kind), "{kind}");
    }
    let failed = events.last().unwrap();
    assert_eq!(failed["type"], "response.failed");
    assert_eq!(failed["response"]["status"], "failed");
    assert_eq!(failed["response"]["error"]["message"], "Cleanup");
}

#[test]
fn failure_without_output_follows_prelude_and_preserves_error_shape() {
    let mut composer = ResponsesComposer::new("fixture-model", "fixture-response");
    let mut frames = composer.open();
    frames.extend(composer.fail("maximum context length exceeded"));
    let events = decode(frames);
    assert_sequence(&events);
    assert_eq!(events.len(), 3);
    assert_eq!(events[2]["type"], "response.failed");
    let (_, expected) = error_json(ClientDialect::OpenAiResponses,
        "maximum context length exceeded");
    assert_eq!(events[2]["response"]["error"], expected["error"]);
    assert_eq!(events[2]["response"]["error"]["code"], "context_length_exceeded");
}

#[test]
fn fresh_streams_number_empty_and_partial_completions_without_resetting_status() {
    for output in [vec![], vec![message("limited", "incomplete")]] {
        let mut response = result(output.clone());
        if !output.is_empty() {
            response.status = "incomplete".into();
            response.incomplete_details = Some(json!({"reason":"max_output_tokens"}));
        }
        let mut composer = ResponsesComposer::new("fixture-model", "fixture-response");
        let mut frames = composer.open();
        frames.extend(composer.push_items(&output, true));
        frames.extend(composer.finish(&response));
        let events = decode(frames);
        assert_sequence(&events);
        let terminal = &events.last().unwrap()["response"];
        assert_eq!(terminal["status"], response.status);
        assert_eq!(terminal["service_tier"], "priority");
        if output.is_empty() {
            assert_eq!(events.len(), 3);
            assert_eq!(terminal["output"], json!([]));
        } else {
            assert_eq!(terminal["incomplete_details"]["reason"], "max_output_tokens");
            assert_eq!(item_event(&events, "response.output_item.done", 0)["item"]["status"],
                "incomplete");
        }
    }
}

#[test]
fn opening_and_terminal_frames_keep_the_same_logical_response_id() {
    for (logical, expected) in [("fixture", "resp_fixture"),
        ("resp_fixture", "resp_fixture"), ("msg_fixture", "resp_msg_fixture")] {
        let mut composer = ResponsesComposer::new("fixture-model", logical);
        let mut frames = composer.open();
        frames.extend(composer.finish(&result(vec![])));
        let events = decode(frames);
        assert_eq!(events.len(), 3);
        assert_sequence(&events);
        for event in events {
            assert_eq!(event["response"]["id"], expected);
        }
    }
}

#[test]
fn heartbeats_preserve_response_identity_text_tools_and_sequence() {
    for logical in ["fixture", "resp_fixture", "msg_fixture"] {
        let first = message("first ", "incomplete");
        let last = message("second", "completed");
        let mut merged = first.clone();
        merge_response_message(&mut merged, &last);
        let mut composer = ResponsesComposer::new("fixture-model", logical);
        let mut frames = composer.open();
        frames.push(composer.heartbeat());
        frames.extend(composer.push_items(&[first], true));
        frames.push(composer.heartbeat());
        frames.extend(composer.push_items(&[last], true));
        frames.push(composer.heartbeat());
        frames.extend(composer.push_items(&tools(), false));
        frames.push(composer.heartbeat());
        let mut output = vec![merged];
        output.extend(tools());
        frames.extend(composer.finish(&result(output)));
        let events = decode(frames);
        assert_sequence(&events);
        let terminal = &events.last().unwrap()["response"];
        assert_eq!(terminal["output"][0]["content"][0]["text"], "first second");
        let pulses: Vec<_> = events.iter().filter(|event|
            event["type"] == "response.in_progress").collect();
        assert_eq!(pulses.len(), 5);
        for pulse in pulses {
            assert_eq!(pulse["response"]["id"], terminal["id"]);
            assert_eq!(pulse["response"]["status"], "in_progress");
            assert!(pulse["response"].get("output").is_none());
        }
        for (index, tool) in tools().iter().enumerate() { assert_tool(&events, index + 1, tool); }
    }
}

#[test]
fn heartbeat_does_not_publish_pending_content_on_failure() {
    let mut composer = ResponsesComposer::new("fixture-model", "fixture");
    let mut frames = composer.open();
    frames.extend(composer.push_items(&[message("partial", "incomplete")], true));
    frames.push(composer.heartbeat());
    frames.push(composer.heartbeat());
    let failed = composer.fail("Cleanup");
    assert_eq!(failed.len(), 1);
    frames.extend(failed);
    let events = decode(frames);
    assert_sequence(&events);
    assert_eq!(events.last().unwrap()["type"], "response.failed");
    assert!(!events.iter().any(|event| matches!(event["type"].as_str(),
        Some("response.completed" | "response.output_item.done" | "response.output_text.done"))));
}
