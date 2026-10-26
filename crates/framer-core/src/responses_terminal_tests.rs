use super::*;

fn partial(output: Vec<Value>, details: Option<Value>) -> AssembledResponses {
    AssembledResponses { id:"upstream-response".into(), created_at:123,
        output, status:"incomplete".into(), incomplete_details:details,
        usage:json!({"input_tokens":7,"output_tokens":11,"total_tokens":18}),
        service_tier:json!("priority"), input_tokens:7, output_tokens:11,
        chunks:2, metered_chunks:0 }
}

fn decode(frames: Vec<String>) -> Vec<Value> {
    frames.iter().enumerate().map(|(index, frame)| {
        let data = frame.lines().find_map(|line| line.strip_prefix("data: ")).unwrap();
        let kind = frame.lines().find_map(|line| line.strip_prefix("event: ")).unwrap();
        let event: Value = serde_json::from_str(data).unwrap();
        assert_eq!(event["type"], kind);
        assert_eq!(event["sequence_number"], index);
        event
    }).collect()
}

fn incomplete(events: &[Value]) -> &Value {
    let terminal = events.last().unwrap();
    assert_eq!(terminal["type"], "response.incomplete");
    assert_eq!(events.iter().filter(|event| matches!(event["type"].as_str(),
        Some("response.completed" | "response.incomplete" | "response.failed"))).count(), 1);
    assert_eq!(terminal["response"]["id"], events[0]["response"]["id"]);
    &terminal["response"]
}

#[test]
fn incomplete_terminal_preserves_content_identity_and_stop_metadata() {
    let output = vec![
        json!({"type":"function_call","id":"fc-supplied","call_id":"call-supplied",
            "name":"read","namespace":"files","arguments":"{}","status":"completed"}),
        json!({"type":"message","id":"message-supplied","role":"assistant",
            "status":"incomplete","extension":{"keep":true},"content":[
                {"type":"output_text","text":"partial α","annotations":[{"type":"future"}]},
                {"type":"refusal","refusal":"restricted portion"}]}),
    ];
    for reason in ["max_output_tokens", "content_filter", "future_reason"] {
        let details = json!({"reason":reason,"extension":{"preserve":true}});
        let response = partial(output.clone(), Some(details.clone()));
        let mut composer = ResponsesComposer::new("fixture-model", "resp_fixture");
        let mut frames = composer.open();
        frames.extend(composer.push_items(&output, true));
        frames.extend(composer.finish(&response));
        let events = decode(frames);
        let full = incomplete(&events);
        assert_eq!(full["status"], "incomplete");
        assert_eq!(full["incomplete_details"], details);
        assert_eq!(full["usage"], response.usage);
        assert_eq!(full["service_tier"], "priority");
        let mut expected = output.clone();
        expected[1]["id"] = full["output"][1]["id"].clone();
        assert_eq!(full["output"], json!(expected));
        let done: Vec<_> = events.iter().filter(|event|
            event["type"] == "response.output_item.done").map(|event| event["item"].clone()).collect();
        assert_eq!(full["output"], json!(done));
    }
}

#[test]
fn empty_incomplete_terminal_preserves_optional_details_and_sequence() {
    for details in [None, Some(Value::Null)] {
        let response = partial(vec![], details.clone());
        let mut composer = ResponsesComposer::new("fixture-model", "fixture");
        let mut frames = composer.open();
        frames.extend(composer.finish(&response));
        let events = decode(frames);
        assert_eq!(events.len(), 3);
        let full = incomplete(&events);
        assert_eq!(full["id"], "resp_fixture");
        assert_eq!(full["status"], "incomplete");
        assert_eq!(full["output"], json!([]));
        assert_eq!(full["incomplete_details"],
            details.unwrap_or_else(|| json!({"reason":"max_output_tokens"})));
        assert_eq!(full["usage"], response.usage);
    }
}
