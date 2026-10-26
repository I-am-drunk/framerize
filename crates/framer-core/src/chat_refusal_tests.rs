use super::*;

fn message(content: Vec<Value>) -> Value {
    json!({"id":"message-fixture","type":"message","role":"assistant",
        "status":"completed","content":content})
}

fn response(output: Vec<Value>) -> AssembledResponses {
    AssembledResponses { id:"response-fixture".into(), created_at:1, output,
        status:"completed".into(), incomplete_details:None, usage:json!({}),
        service_tier:json!("default"), input_tokens:7, output_tokens:11,
        chunks:1, metered_chunks:0 }
}

fn decode(frames: Vec<String>) -> Vec<Value> {
    frames.iter().filter_map(|frame| {
        let data = frame.strip_prefix("data: ").unwrap();
        if data == "[DONE]" { None } else { Some(serde_json::from_str(data).unwrap()) }
    }).collect()
}

#[test]
fn refusal_only_reply_is_explicit_in_chat_json() {
    for refusal in ["I cannot complete this request.", ""] {
        let r = response(vec![message(vec![json!({"type":"refusal","refusal":refusal})])]);
        let rendered = responses_to_chat_completion(&r, "fixture-model");
        assert_eq!(rendered["choices"][0]["message"],
            json!({"role":"assistant","content":null,"refusal":refusal}));
        assert_eq!(rendered["choices"][0]["finish_reason"], "stop");
        assert_eq!(rendered["usage"],
            json!({"prompt_tokens":7,"completion_tokens":11,"total_tokens":18}));
    }
}

#[test]
fn assistant_refusal_survives_the_next_chat_request() {
    let refusal = json!({"type":"refusal","refusal":"restricted"});
    let cases = [
        (json!({"role":"assistant","content":null,"refusal":"restricted"}), vec![refusal.clone()]),
        (json!({"role":"assistant","content":"before","refusal":"restricted"}),
            vec![json!({"type":"output_text","text":"before"}), refusal.clone()]),
        (json!({"role":"assistant","content":[{"type":"text","text":"before"},
            refusal.clone(), {"type":"text","text":"after"}]}),
            vec![json!({"type":"output_text","text":"before"}), refusal,
                json!({"type":"output_text","text":"after"})]),
    ];
    for (assistant, expected) in cases {
        let converted = chat_to_responses_request(&json!({"model":"fixture-model",
            "messages":[{"role":"user","content":"prompt"},assistant]})).unwrap();
        assert_eq!(converted["input"].as_array().unwrap().len(), 2);
        assert_eq!(converted["input"][1]["content"], json!(expected));
    }
}

#[test]
fn streamed_text_and_refusal_keep_their_order_across_chunks() {
    let first = message(vec![json!({"type":"output_text","text":"before"}),
        json!({"type":"refusal","refusal":"cannot "})]);
    let last = message(vec![json!({"type":"refusal","refusal":"continue"}),
        json!({"type":"output_text","text":"after"})]);
    let r = response(vec![first.clone(), last.clone()]);
    let mut composer = ChatComposer::new("fixture-model", "response-fixture");
    let mut events = decode(composer.push_items(&[first], true));
    events.extend(decode(composer.push_items(&[last], false)));
    let deltas: Vec<Value> = events.iter().map(|event| event["choices"][0]["delta"].clone()).collect();
    assert_eq!(deltas, vec![json!({"content":"before"}), json!({"refusal":"cannot "}),
        json!({"refusal":"continue"}), json!({"content":"after"})]);
    let finished = composer.finish(&r, false);
    assert_eq!(finished.last().unwrap(), "data: [DONE]");
    assert_eq!(decode(finished)[0]["choices"][0]["finish_reason"], "stop");
    let rendered = responses_to_chat_completion(&r, "fixture-model");
    assert_eq!(rendered["choices"][0]["message"]["content"], "beforeafter");
    assert_eq!(rendered["choices"][0]["message"]["refusal"], "cannot continue");
}

#[test]
fn refusal_with_a_tool_preserves_call_and_result_history() {
    let call = json!({"type":"function_call","id":"fc-fixture","call_id":"call-fixture",
        "name":"fixture","arguments":"{\"value\":42}"});
    let r = response(vec![message(vec![json!({"type":"refusal","refusal":"restricted"})]), call]);
    let rendered = responses_to_chat_completion(&r, "fixture-model");
    let assistant = &rendered["choices"][0]["message"];
    assert_eq!(assistant["refusal"], "restricted");
    assert_eq!(assistant["tool_calls"][0], json!({"id":"call-fixture","type":"function",
        "function":{"name":"fixture","arguments":"{\"value\":42}"}}));
    assert_eq!(rendered["choices"][0]["finish_reason"], "tool_calls");
    let converted = chat_to_responses_request(&json!({"model":"fixture-model","messages":[
        {"role":"user","content":"prompt"}, assistant,
        {"role":"tool","tool_call_id":"call-fixture","content":"result"}]})).unwrap();
    assert_eq!(converted["input"].as_array().unwrap().len(), 4);
    assert_eq!(converted["input"][1]["content"], json!([{"type":"refusal","refusal":"restricted"}]));
    assert_eq!(converted["input"][2]["call_id"], "call-fixture");
    assert_eq!(converted["input"][3], json!({"type":"function_call_output",
        "call_id":"call-fixture","output":"result"}));
}

#[test]
fn text_only_and_empty_chat_replies_do_not_acquire_a_refusal() {
    for (text, status, finish) in [("answer", "completed", "stop"),
        ("partial", "incomplete", "length"), ("", "completed", "stop")] {
        let mut r = response(vec![message(vec![json!({"type":"output_text","text":text})])]);
        r.status = status.into();
        let rendered = responses_to_chat_completion(&r, "fixture-model");
        assert_eq!(rendered["choices"][0]["message"]["content"], text);
        assert!(rendered["choices"][0]["message"].get("refusal").is_none());
        let mut composer = ChatComposer::new("fixture-model", "response-fixture");
        let events = decode(composer.push_items(&r.output, false));
        assert!(events.iter().all(|event| event["choices"][0]["delta"].get("refusal").is_none()));
        assert_eq!(decode(composer.finish(&r, false))[0]["choices"][0]["finish_reason"], finish);
    }
}
