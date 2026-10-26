use crate::{auth::AiToken, chain::responses::complete_long, transport::Transport};
use async_trait::async_trait;
use framer_core::convert::{self, AssembledResponses, ChainEvent, ClientDialect, ResponsesComposer};
use framer_core::ids::{AccountId, FramerUserId, ProjectId};
use framer_core::port::{BrowserBridge, CastleToken, ChainConfig, Session};
use serde_json::{json, Value};
use std::{collections::VecDeque, sync::Mutex};

struct Fixture {
    replies: Mutex<VecDeque<Value>>,
    requests: Mutex<Vec<Value>>,
}

struct Observed {
    response: AssembledResponses,
    events: Vec<Value>,
    chat_events: Vec<Value>,
    requests: Vec<Value>,
}

fn run(replies: Vec<Value>) -> Observed {
    run_with_payload(replies, json!({"model":"gpt-6-astra","input":"fixture"}))
}

fn run_with_payload(replies: Vec<Value>, payload: Value) -> Observed {
    tokio::runtime::Builder::new_current_thread().enable_all().start_paused(true)
        .build().unwrap().block_on(async {
        let account = AccountId(uuid::Uuid::from_u128(41));
        let session = Session { active_user: FramerUserId("fixture-user".into()),
            cookie_header:"fixture-only".into() };
        let cache = Mutex::new(vec![(account,
            AiToken::for_session(&session, "fixture-only".into(), i64::MAX))]);
        let browser = Fixture { replies:Mutex::new(replies.into()),
            requests:Mutex::new(Vec::new()) };
        let mut response_composer = ResponsesComposer::new("fixture-model", "fixture-response");
        let frames = Mutex::new(response_composer.open());
        let composer = Mutex::new(Some(response_composer));
        let chat = Mutex::new(Some(convert::ChatComposer::new("fixture-model", "fixture-response")));
        let chat_frames = Mutex::new(convert::stream_open(
            ClientDialect::OpenAiChat, "fixture-model", "fixture-response"));
        let callback = |event| {
            if let ChainEvent::ResponsesItems { items, continues_text } = event {
                let next = composer.lock().unwrap().as_mut().unwrap()
                    .push_items(&items, continues_text);
                frames.lock().unwrap().extend(next);
                chat_frames.lock().unwrap().extend(chat.lock().unwrap().as_mut().unwrap()
                    .push_items(&items, continues_text));
            }
        };
        let config = ChainConfig { chunk_tokens:600, max_chunks:3,
            max_total_tokens:Some(2048), metered_fallback:false, ..ChainConfig::default() };
        let response = complete_long(&Transport::with_base("http://127.0.0.1:9".into()),
            &cache, &session, &browser, &account, &ProjectId("fixture-project".into()),
            payload, &config,
            "fixture-chat", Some(&callback)).await.unwrap();
        assert!(browser.replies.lock().unwrap().is_empty());
        frames.lock().unwrap().extend(composer.lock().unwrap().take().unwrap().finish(&response));
        chat_frames.lock().unwrap().extend(chat.lock().unwrap().take().unwrap().finish(&response, true));
        let events = frames.lock().unwrap().iter().map(|frame| {
            let data = frame.lines().find_map(|line| line.strip_prefix("data: ")).unwrap();
            serde_json::from_str(data).unwrap()
        }).collect();
        let chat_events = chat_frames.lock().unwrap().iter().filter_map(|frame| {
            let data = frame.lines().find_map(|line| line.strip_prefix("data: ")).unwrap();
            if data == "[DONE]" { None } else { Some(serde_json::from_str(data).unwrap()) }
        }).collect();
        Observed { response, events, chat_events, requests:browser.requests.into_inner().unwrap() }
    })
}

#[async_trait]
impl BrowserBridge for Fixture {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        panic!("unexpected mint")
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("unexpected login")
    }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> {
        panic!("unexpected login")
    }
    async fn seed_session(&self, account: &AccountId, cookie: &str) -> Result<(), String> {
        assert_eq!(account.0, uuid::Uuid::from_u128(41));
        assert_eq!(cookie, "fixture-only");
        Ok(())
    }
    async fn ai_post(&self, account: &AccountId, path: &str, bearer: &str, body: &Value)
        -> Result<(u16, Option<String>, String), String>
    {
        assert_eq!(account.0, uuid::Uuid::from_u128(41));
        assert_eq!(path, "/ai/v3/responses/");
        assert_eq!(bearer, "fixture-only");
        assert_eq!(body["request"]["stream"], false);
        assert_eq!(body["projectId"], "fixture-project");
        assert_eq!(body["sessionId"], "fixture-chat");
        self.requests.lock().unwrap().push(body.clone());
        let reply = self.replies.lock().unwrap().pop_front().expect("unexpected attempt");
        Ok((200, None, reply.to_string()))
    }
}

fn message(id: &str, text: &str, status: &str) -> Value {
    json!({"id":id,"type":"message","role":"assistant","status":status,
        "content":[{"type":"output_text","text":text,"annotations":[]}]})
}

fn reply(output: Vec<Value>, status: &str) -> Value {
    let mut body = json!({"id":"upstream-response","status":status,"output":output,
        "usage":{"input_tokens":5,"output_tokens":100}});
    if status == "incomplete" {
        body["incomplete_details"] = json!({"reason":"max_output_tokens"});
    }
    body
}

fn selected<'a>(observed: &'a Observed, kind: &str) -> Vec<&'a Value> {
    observed.events.iter().filter(|event| event["type"] == kind).collect()
}

fn assert_event_sequence(observed: &Observed) {
    for (index, event) in observed.events.iter().enumerate() {
        assert_eq!(event["sequence_number"].as_u64(), Some(index as u64), "{event}");
    }
}

fn event_tool(kind: &str, field: &str) -> Value {
    let mut item = json!({"type":kind,"id":format!("{kind}-item"),
        "call_id":format!("call/{kind}"),"name":"run","namespace":"tools",
        "status":"completed","extension":{"retained":true}});
    item[field] = json!("{\"n\":1}");
    item
}

fn assert_anonymous_tool_events(observed: &Observed, index: usize, source: &Value) {
    let terminals = selected(observed, "response.completed");
    let item = &terminals[0]["response"]["output"][index];
    assert!(!item["id"].as_str().unwrap().is_empty());
    let mut expected = source.clone();
    expected["id"] = item["id"].clone();
    assert_eq!(*item, expected);
    assert_eq!(observed.response.output[index], *source);
    assert_eq!(selected(observed, "response.output_item.added")[index]["item"]["id"], item["id"]);
    assert_eq!(selected(observed, "response.output_item.done")[index]["item"], *item);
    let stem = if source["type"] == "function_call" {
        "response.function_call_arguments"
    } else { "response.custom_tool_call_input" };
    for suffix in ["delta", "done"] {
        let events = selected(observed, &format!("{stem}.{suffix}"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["output_index"], index);
        assert_eq!(events[0]["item_id"], item["id"]);
    }
}

#[test]
fn opaque_output_keeps_chain_messages_and_tool_event_positions() {
    let search = json!({"type":"web_search_call","id":"search-id","status":"completed",
        "action":{"type":"search","query":"fixture"},"extension":{"keep":true}});
    let output = vec![message("first", "before", "completed"), search.clone(),
        message("last", "after", "completed"), event_tool("function_call", "arguments")];
    let observed = run(vec![reply(output.clone(), "completed")]);
    assert_eq!(observed.response.output, output);
    assert_event_sequence(&observed);
    let added = selected(&observed, "response.output_item.added");
    let done = selected(&observed, "response.output_item.done");
    assert_eq!((added.len(), done.len()), (4, 4));
    let terminals = selected(&observed, "response.completed");
    let final_output = &terminals[0]["response"]["output"];
    for index in 0..4 {
        assert_eq!(added[index]["output_index"], index);
        assert_eq!(done[index]["output_index"], index);
        assert_eq!(done[index]["item"], final_output[index]);
    }
    assert_eq!(added[1]["item"], search);
    let arguments = selected(&observed, "response.function_call_arguments.done");
    assert_eq!(arguments[0]["output_index"], 3);
    assert_eq!(arguments[0]["item_id"], output[3]["id"]);
}

#[test]
fn anonymous_tool_event_ids_leave_chain_output_and_history_unchanged() {
    let mut tools = vec![event_tool("function_call", "arguments"),
        event_tool("custom_tool_call", "input")];
    for item in &mut tools {
        item.as_object_mut().unwrap().remove("id");
        item.as_object_mut().unwrap().remove("status");
    }
    let first = message("first", "before ", "incomplete");
    let mut second = vec![message("last", "tools", "completed")];
    second.extend(tools.clone());
    let observed = run(vec![reply(vec![first.clone()], "incomplete"),
        reply(second, "completed")]);
    assert_eq!(observed.requests.len(), 2);
    assert!(observed.requests[1]["request"]["input"].as_array().unwrap().contains(&first));
    assert_anonymous_tool_events(&observed, 1, &tools[0]);
    assert_anonymous_tool_events(&observed, 2, &tools[1]);
    let terminals = selected(&observed, "response.completed");
    let output = &terminals[0]["response"]["output"];
    assert_ne!(output[1]["id"], output[2]["id"]);
    assert_event_sequence(&observed);
}

#[test]
fn supplied_tool_event_ids_and_sequence_survive_chain_continuation() {
    for (kind, field, stem) in [
        ("function_call", "arguments", "response.function_call_arguments"),
        ("custom_tool_call", "input", "response.custom_tool_call_input"),
    ] {
        let tool = event_tool(kind, field);
        let observed = run(vec![
            reply(vec![message("first", "before ", "incomplete")], "incomplete"),
            reply(vec![message("last", "tool", "completed"), tool.clone()], "completed")]);
        assert_eq!(observed.requests.len(), 2);
        for (suffix, payload) in [("delta", "delta"), ("done", field)] {
            let events = selected(&observed, &format!("{stem}.{suffix}"));
            assert_eq!(events.len(), 1);
            assert_eq!(events[0]["output_index"], 1);
            assert_eq!(events[0]["item_id"], tool["id"]);
            assert_eq!(events[0][payload], tool[field]);
        }
        assert_eq!(selected(&observed, "response.output_item.added")[1]["item"]["id"], tool["id"]);
        assert_eq!(selected(&observed, "response.output_item.done")[1]["item"], tool);
        assert_eq!(selected(&observed, "response.completed")[0]["response"]["output"][1], tool);
        assert_eq!(observed.response.output[1], tool);
        assert_event_sequence(&observed);
    }
}

#[test]
fn completed_continuation_is_one_message_from_chain_through_terminal_sse() {
    let first = message("first-upstream", "prefix ", "incomplete");
    let observed = run(vec![reply(vec![first.clone()], "incomplete"),
        reply(vec![message("last-upstream", "suffix", "completed")], "completed")]);
    assert_eq!(observed.requests.len(), 2);
    assert!(observed.requests[1]["request"]["input"].as_array().unwrap().contains(&first));
    assert_eq!(observed.response.output_tokens, 200);
    assert_eq!(observed.response.output, vec![message("first-upstream", "prefix suffix", "completed")]);
    let added = selected(&observed, "response.output_item.added");
    let done = selected(&observed, "response.output_item.done");
    assert_eq!((added.len(), done.len()), (1, 1));
    let completed = selected(&observed, "response.completed");
    assert_eq!(completed.len(), 1);
    let full = &completed[0]["response"];
    assert_eq!(full["id"], observed.events[0]["response"]["id"]);
    assert_eq!(added[0]["item"]["id"], full["output"][0]["id"]);
    assert_eq!(done[0]["item"], full["output"][0]);
    assert_eq!(full["output"][0]["content"][0]["text"], "prefix suffix");
}

#[test]
fn continued_text_and_tool_keep_one_ordered_terminal_snapshot() {
    for (kind, field, input) in [("function_call", "arguments", "{\"n\":1}"),
        ("custom_tool_call", "input", "fixture();")] {
        let mut tool = json!({"id":"tool-fixture","type":kind,"call_id":"call-fixture",
            "name":"run","namespace":"tools","status":"completed"});
        tool[field] = json!(input);
        let observed = run(vec![
            reply(vec![message("first", "before ", "incomplete")], "incomplete"),
            reply(vec![message("last", "tool", "completed"), tool.clone()], "completed")]);
        assert_eq!(observed.requests.len(), 2);
        let added = selected(&observed, "response.output_item.added");
        let done = selected(&observed, "response.output_item.done");
        assert_eq!((added.len(), done.len()), (2, 2));
        let finals = selected(&observed, "response.completed");
        let output = finals[0]["response"]["output"].as_array().unwrap();
        assert_eq!(output.len(), 2);
        for i in 0..2 {
            assert_eq!(done[i]["item"], output[i]);
            assert_eq!(added[i]["item"]["id"], output[i]["id"]);
        }
        assert_eq!(output[0]["content"][0]["text"], "before tool");
        assert_eq!(output[1], tool);
    }
}

#[test]
fn annotated_continuation_preserves_part_boundaries_and_offsets() {
    let first = message("first", "prefix 🚀 ", "incomplete");
    let mut last = message("last", "word", "completed");
    let citation = json!({"type":"url_citation","start_index":0,"end_index":4,
        "url":"https://example.invalid/fixture","title":"fixture"});
    last["content"][0]["annotations"] = json!([citation]);
    let expected = json!([first["content"][0], last["content"][0]]);
    let observed = run(vec![reply(vec![first], "incomplete"), reply(vec![last], "completed")]);
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.response.output[0]["content"], expected);
    let parts: Vec<Value> = selected(&observed, "response.content_part.done").iter()
        .map(|event| event["part"].clone()).collect();
    assert_eq!(json!(parts), expected);
    let annotations = selected(&observed, "response.output_text.annotation.added");
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0]["content_index"], 1);
    assert_eq!(annotations[0]["annotation"], citation);
    assert_eq!(selected(&observed, "response.output_item.done")[0]["item"],
        selected(&observed, "response.completed")[0]["response"]["output"][0]);
}

#[test]
fn refusal_continuation_preserves_typed_content_and_order() {
    let first = message("first", "before", "incomplete");
    let refusal = json!({"type":"refusal","refusal":"restricted portion"});
    let mut last = message("last", "after", "completed");
    last["content"].as_array_mut().unwrap().insert(0, refusal.clone());
    let expected = json!([first["content"][0], refusal, last["content"][1]]);
    let observed = run(vec![reply(vec![first], "incomplete"), reply(vec![last], "completed")]);
    assert_eq!(observed.response.output[0]["content"], expected);
    let parts: Vec<Value> = selected(&observed, "response.content_part.done").iter()
        .map(|event| event["part"].clone()).collect();
    assert_eq!(json!(parts), expected);
    let refusals = selected(&observed, "response.refusal.done");
    assert_eq!(refusals.len(), 1);
    assert_eq!(refusals[0]["content_index"], 1);
    assert_eq!(refusals[0]["refusal"], "restricted portion");
    assert_eq!(selected(&observed, "response.output_item.done")[0]["item"],
        selected(&observed, "response.completed")[0]["response"]["output"][0]);
}

#[test]
fn chat_refusal_survives_the_chain_json_stream_and_next_request() {
    let mut item = message("upstream", "", "completed");
    item["content"] = json!([{"type":"refusal","refusal":"cannot complete this request"}]);
    let observed = run(vec![reply(vec![item], "completed")]);
    assert_eq!(observed.requests.len(), 1);
    let rendered = convert::responses_to_chat_completion(&observed.response, "fixture-model");
    let assistant = &rendered["choices"][0]["message"];
    assert_eq!(assistant["content"], Value::Null);
    assert_eq!(assistant["refusal"], "cannot complete this request");
    let refusals: Vec<_> = observed.chat_events.iter().filter_map(|event|
        event["choices"][0]["delta"]["refusal"].as_str()).collect();
    assert_eq!(refusals, vec!["cannot complete this request"]);
    let history = convert::chat_to_responses_request(&json!({"model":"fixture-model",
        "messages":[{"role":"user","content":"prompt"},assistant]})).unwrap();
    assert_eq!(history["input"][1]["content"],
        json!([{"type":"refusal","refusal":"cannot complete this request"}]));
    assert_eq!(rendered["choices"][0]["finish_reason"], "stop");
}

#[test]
fn chat_continuation_keeps_refusal_before_the_tool_call() {
    let first = message("first", "accepted prefix", "incomplete");
    let mut last = message("last", "", "completed");
    last["content"] = json!([{"type":"refusal","refusal":"restricted portion"}]);
    let call = json!({"type":"function_call","id":"fc-fixture","call_id":"call-fixture",
        "name":"fixture","arguments":"{\"value\":42}","status":"completed"});
    let observed = run(vec![reply(vec![first], "incomplete"),
        reply(vec![last, call], "completed")]);
    assert_eq!(observed.requests.len(), 2);
    let rendered = convert::responses_to_chat_completion(&observed.response, "fixture-model");
    let assistant = &rendered["choices"][0]["message"];
    assert_eq!(assistant["content"], "accepted prefix");
    assert_eq!(assistant["refusal"], "restricted portion");
    assert_eq!(assistant["tool_calls"][0]["id"], "call-fixture");
    assert_eq!(rendered["choices"][0]["finish_reason"], "tool_calls");
    let refusal_index = observed.chat_events.iter().position(|event|
        event["choices"][0]["delta"]["refusal"] == "restricted portion").unwrap();
    let tool_index = observed.chat_events.iter().position(|event|
        event["choices"][0]["delta"]["tool_calls"][0]["id"] == "call-fixture").unwrap();
    assert!(refusal_index < tool_index);
    let usage = &observed.chat_events.last().unwrap()["usage"];
    assert_eq!(usage["completion_tokens"], 200);
}

#[test]
fn chat_content_filter_is_terminal_and_visible_after_continuation() {
    let first = message("first", "accepted prefix", "incomplete");
    let mut last = message("last", "", "incomplete");
    last["content"] = json!([{"type":"refusal","refusal":"restricted portion"}]);
    let mut filtered = reply(vec![last], "incomplete");
    filtered["incomplete_details"] = json!({"reason":"content_filter"});
    let observed = run(vec![reply(vec![first], "incomplete"), filtered]);
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.response.status, "incomplete");
    assert_eq!(observed.response.incomplete_details, Some(json!({"reason":"content_filter"})));
    let rendered = convert::responses_to_chat_completion(&observed.response, "fixture-model");
    assert_eq!(rendered["choices"][0]["finish_reason"], "content_filter");
    let reasons: Vec<_> = observed.chat_events.iter().filter_map(|event|
        event["choices"][0]["finish_reason"].as_str()).collect();
    assert_eq!(reasons, vec!["content_filter"]);
    assert_eq!(rendered["choices"][0]["message"]["content"], "accepted prefix");
    assert_eq!(rendered["choices"][0]["message"]["refusal"], "restricted portion");
}

#[test]
fn chat_tool_strictness_reaches_each_continuation_request() {
    for strict in [Some(true), Some(false), None] {
        let expected = strict.map(|value| json!(value));
        let schema = json!({"type":"object","properties":{},"additionalProperties":false});
        let mut function = json!({"name":"fixture","parameters":schema});
        if let Some(value) = &expected { function["strict"] = value.clone(); }
        let payload = convert::chat_to_responses_request(&json!({"model":"gpt-6-astra",
            "messages":[{"role":"user","content":"fixture"}],
            "tools":[{"type":"function","function":function}]})).unwrap();
        let observed = run_with_payload(vec![
            reply(vec![message("first", "prefix ", "incomplete")], "incomplete"),
            reply(vec![message("last", "suffix", "completed")], "completed"),
        ], payload);
        assert_eq!(observed.requests.len(), 2);
        for request in &observed.requests {
            let tool = &request["request"]["tools"][0];
            assert_eq!(tool.get("strict"), expected.as_ref());
            assert_eq!(tool["parameters"], schema);
        }
        assert_eq!(observed.response.output[0]["content"][0]["text"], "prefix suffix");
    }
}

#[test]
fn chat_json_schema_reaches_each_continuation_request() {
    let schema = json!({"type":"object", "properties":{"value":{"type":"integer"}},
        "required":["value"], "additionalProperties":false});
    let format = json!({"name":"result", "description":"fixture result", "strict":true, "schema":schema});
    let payload = convert::chat_to_responses_request(&json!({"model":"gpt-6-astra",
        "messages":[{"role":"user","content":"structured answer"}],
        "response_format":{"type":"json_schema","json_schema":format}})).unwrap();
    let observed = run_with_payload(vec![
        reply(vec![message("first", "{\"value\":", "incomplete")], "incomplete"),
        reply(vec![message("last", "42}", "completed")], "completed"),
    ], payload);
    let mut expected = format;
    expected["type"] = json!("json_schema");
    assert_eq!(observed.requests.len(), 2);
    for request in &observed.requests {
        assert_eq!(request["request"]["text"]["format"], expected);
        assert!(request["request"].get("response_format").is_none());
    }
    assert_eq!(observed.response.output[0]["content"][0]["text"], "{\"value\":42}");
}
