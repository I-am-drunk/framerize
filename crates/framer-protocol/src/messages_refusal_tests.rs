use super::*;

fn refusal() -> AssembledResponse {
    AssembledResponse {
        id: "msg_refused".into(), content: vec![], stop_reason: "refusal".into(),
        stop_sequence: None, stop_details: Some(json!({"type":"refusal",
            "category":"reasoning_extraction", "explanation":"Provider explanation",
            "future_field":{"preserved":true}})),
        usage: json!({"input_tokens":20,"output_tokens":1}),
        input_tokens:20, output_tokens:1, chunks:1, metered_chunks:0,
    }
}

fn values(frames: Vec<String>) -> Vec<Value> {
    frames.iter().filter_map(|frame| frame.lines()
        .find_map(|line| line.strip_prefix("data: ")))
        .filter(|text| *text != "[DONE]")
        .map(|text| serde_json::from_str(text).unwrap()).collect()
}

fn check_response_refusal(events: &[Value], output_index: usize) {
    let refusal: Vec<_> = events.iter().filter(|v|
        v["type"] == "response.refusal.delta" || v["type"] == "response.refusal.done").collect();
    assert_eq!(refusal.len(), 2);
    assert_eq!(refusal[0]["delta"], "Provider explanation");
    assert_eq!(refusal[1]["refusal"], "Provider explanation");
    let completed = &events.last().unwrap()["response"];
    let item = &completed["output"][output_index];
    assert_eq!(item["content"][0]["type"], "refusal");
    for event in refusal {
        assert_eq!(event["output_index"], output_index);
        assert_eq!(event["item_id"], item["id"]);
    }
    assert_eq!(completed["stop_details"], refusal_details());
    assert!(events.iter().any(|v| v["type"] == "response.output_item.done"
        && v["item"] == *item));
}

fn refusal_details() -> Value { refusal().stop_details.unwrap() }

#[test]
fn later_refusal_preserves_the_published_prefix_and_its_own_item() {
    let mut response = refusal();
    response.content.push(json!({"type":"text","text":"Earlier accepted output"}));
    let dialect = ClientDialect::OpenAiResponses;
    let mut composer = SseComposer::new(dialect, "local", "fixture");
    let mut frames = composer.push_chunk(&response.content);
    frames.extend(composer.finish(&response, "fixture", false));
    let events = values(frames);
    check_response_refusal(&events, 1);
    assert_eq!(events.last().unwrap()["response"]["output"][0]["content"][0]["text"],
        "Earlier accepted output");
    assert_eq!(events.iter().filter(|v| v["type"] == "response.output_item.done").count(), 2);
}

#[test]
fn missing_explanation_does_not_invent_a_policy_cause() {
    let mut response = refusal();
    response.stop_details = None;
    response.content.push(json!({"type":"text","text":"Earlier accepted output"}));
    let body = render_json(ClientDialect::OpenAiChat, &response, "fixture");
    assert_eq!(body["choices"][0]["message"]["refusal"],
        "Upstream refused the request without structured category or explanation details.");
    response.stop_reason = "end_turn".into();
    let body = render_json(ClientDialect::OpenAiChat, &response, "fixture");
    assert!(body["choices"][0]["message"].get("refusal").is_none());
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
}

#[test]
fn responses_json_and_both_streams_emit_a_real_refusal_part() {
    let response = refusal();
    let dialect = ClientDialect::OpenAiResponses;
    let body = render_json(dialect, &response, "fixture");
    assert_eq!(body["output"][0]["content"][0],
        json!({"type":"refusal","refusal":"Provider explanation"}));
    let mut composer = SseComposer::new(dialect, "local", "fixture");
    for frames in [render_sse(dialect, &response, "fixture", false),
        composer.finish(&response, "fixture", false)] {
        check_response_refusal(&values(frames), 0);
    }
}

#[test]
fn chat_json_and_both_streams_expose_refusal_and_original_details() {
    let response = refusal();
    let dialect = ClientDialect::OpenAiChat;
    let body = render_json(dialect, &response, "fixture");
    let choice = &body["choices"][0];
    assert_eq!(choice["finish_reason"], "content_filter");
    assert_eq!(choice["message"]["refusal"], "Provider explanation");
    assert_eq!(choice["stop_details"], response.stop_details.clone().unwrap());
    let mut composer = SseComposer::new(dialect, "local", "fixture");
    for frames in [render_sse(dialect, &response, "fixture", true),
        composer.finish(&response, "fixture", true)] {
        assert_eq!(frames.last().unwrap(), "data: [DONE]");
        let events = values(frames);
        let end = events.iter().find(|v|
            v["choices"][0]["finish_reason"] == "content_filter").unwrap();
        assert_eq!(end["choices"][0]["delta"]["refusal"], "Provider explanation");
        assert_eq!(end["choices"][0]["stop_details"], choice["stop_details"]);
    }
}

#[test]
fn anthropic_json_and_both_streams_preserve_refusal_details() {
    let response = refusal();
    let dialect = ClientDialect::AnthropicMessages;
    let body = render_json(dialect, &response, "fixture");
    assert_eq!(body["content"], json!([]));
    assert_eq!(body["stop_reason"], "refusal");
    assert_eq!(body["stop_details"], response.stop_details.clone().unwrap());
    let mut composer = SseComposer::new(dialect, "local", "fixture");
    for frames in [render_sse(dialect, &response, "fixture", false),
        composer.finish(&response, "fixture", false)] {
        let events = values(frames);
        let delta = events.iter().find(|v| v["type"] == "message_delta").unwrap();
        assert_eq!(delta["delta"]["stop_details"], body["stop_details"]);
        assert_eq!(delta["delta"]["stop_reason"], "refusal");
        assert_eq!(events.last().unwrap()["type"], "message_stop");
    }
}
