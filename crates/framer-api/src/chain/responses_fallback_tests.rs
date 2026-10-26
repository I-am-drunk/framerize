use super::*;
use async_trait::async_trait;
use framer_core::ids::FramerUserId;
use framer_core::port::CastleToken;
use std::collections::VecDeque;

struct Reply {
    streamed: bool,
    body: Option<Value>,
    at_budget: bool,
}

fn response(output: Value, status: &str) -> Value {
    json!({"id":"response-fixture", "created_at":9, "status":status,
        "output":[output], "service_tier":"priority",
        "incomplete_details":if status == "incomplete" {
            json!({"reason":"max_output_tokens"})
        } else { Value::Null },
        "usage":{"input_tokens":11, "output_tokens":7}})
}

fn tool(kind: &str, complete: bool) -> Value {
    let mut item = json!({"type":kind, "id":"item-fixture", "call_id":"call-fixture",
        "name":"fixture", "status":"completed"});
    let key = if kind == "function_call" { "arguments" } else { "input" };
    item[key] = json!(if complete { "{\"x\":1}" } else { "{\"x\":" });
    item
}

fn fallback(body: Value, at_budget: bool) -> Vec<Reply> {
    vec![Reply { streamed:false, body:None, at_budget:false },
        Reply { streamed:false, body:None, at_budget:false },
        Reply { streamed:true, body:Some(body), at_budget }]
}

struct Fixture {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Value>>,
    refunds: Mutex<usize>,
}

#[async_trait]
impl BrowserBridge for Fixture {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> { panic!("unexpected mint") }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> { panic!("unexpected login") }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> { panic!("unexpected login") }
    async fn seed_session(&self, account: &AccountId, cookie: &str) -> Result<(), String> {
        assert_eq!(*account, AccountId(uuid::Uuid::from_u128(1)));
        assert_eq!(cookie, "fixture-only");
        Ok(())
    }
    async fn ai_post(&self, account: &AccountId, path: &str, bearer: &str, body: &Value)
        -> Result<(u16, Option<String>, String), String>
    {
        assert_eq!(*account, AccountId(uuid::Uuid::from_u128(1)));
        assert_eq!(bearer, "fixture-only");
        assert_eq!(body["projectId"], "fixture-project");
        assert_eq!(body["sessionId"], "fixture-chat");
        if path == "/ai/v3/refund/" {
            *self.refunds.lock().unwrap() += 1;
            return Ok((200, None, json!({"granted":true}).to_string()));
        }
        assert_eq!(path, "/ai/v3/responses/");
        self.requests.lock().unwrap().push(body["request"].clone());
        let reply = self.replies.lock().unwrap().pop_front().expect("unexpected attempt");
        assert_eq!(body["request"]["stream"], reply.streamed);
        let Some(mut response) = reply.body else {
            return Err("fixture transport connection closed".into());
        };
        if reply.at_budget {
            response["usage"]["output_tokens"] = body["request"]["max_output_tokens"].clone();
        }
        let text = if reply.streamed {
            format!("data: {}\n\n", json!({"type":"response.completed", "response":response}))
        } else { response.to_string() };
        Ok((200, None, text))
    }
}

struct Run {
    result: Result<AssembledResponses, String>,
    requests: Vec<Value>,
    events: Vec<(Vec<Value>, bool)>,
    refunds: usize,
}

fn config() -> ChainConfig {
    ChainConfig { chunk_tokens:1500, max_total_tokens:Some(4096), max_chunks:8,
        metered_fallback:true, ..ChainConfig::default() }
}

fn run(replies: Vec<Reply>, cfg: ChainConfig) -> Run {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let account = AccountId(uuid::Uuid::from_u128(1));
        let session = Session { active_user:FramerUserId("fixture-user".into()), cookie_header:"fixture-only".into() };
        let cache = Mutex::new(vec![(account, AiToken::for_session(&session, "fixture-only".into(), i64::MAX))]);
        let browser = Fixture { replies:Mutex::new(replies.into()), requests:Mutex::new(Vec::new()), refunds:Mutex::new(0) };
        let events = Mutex::new(Vec::new());
        let callback = |event| {
            if let ChainEvent::ResponsesItems { items, continues_text } = event {
                events.lock().unwrap().push((items, continues_text));
            }
        };
        let result = complete_long(&Transport::with_base("http://127.0.0.1:9".into()),
            &cache, &session, &browser, &account, &ProjectId("fixture-project".into()),
            json!({"model":"gpt-6-astra", "input":"fixture", "service_tier":"priority",
                "reasoning":{"effort":"max"}, "tools":[{"type":"custom", "name":"fixture"}]}),
            &cfg, "fixture-chat", Some(&callback)).await;
        assert!(browser.replies.lock().unwrap().is_empty(), "expected additional attempts");
        Run { result, requests:browser.requests.into_inner().unwrap(),
            events:events.into_inner().unwrap(), refunds:browser.refunds.into_inner().unwrap() }
    })
}

fn recovered_tool(kind: &str) {
    let bad = tool(kind, false);
    let good = tool(kind, true);
    let mut replies = fallback(response(bad.clone(), "completed"), true);
    replies.push(Reply { streamed:false, body:Some(response(good.clone(), "completed")), at_budget:false });
    let actual = run(replies, config());
    let result = actual.result.unwrap();
    assert_eq!(result.output, vec![good.clone()]);
    assert_eq!(actual.events, vec![(vec![good], false)]);
    assert_eq!((result.chunks, result.metered_chunks, actual.refunds), (2, 1, 1));
    assert_eq!(result.output_tokens, 7);
    assert_eq!(actual.requests.len(), 4);
    assert_eq!(actual.requests[1]["max_output_tokens"], actual.requests[2]["max_output_tokens"]);
    assert!(!actual.requests[3]["input"].as_array().unwrap().contains(&bad));
    assert_eq!(actual.requests[0]["tools"], actual.requests[3]["tools"]);
    assert_eq!(actual.requests[0]["reasoning"], actual.requests[3]["reasoning"]);
}

#[test]
fn metered_custom_tool_at_retry_budget_is_withheld_and_repaired() {
    recovered_tool("custom_tool_call");
}

#[test]
fn metered_function_tool_at_retry_budget_is_withheld_and_repaired() {
    recovered_tool("function_call");
}

#[test]
fn final_metered_chunk_refuses_an_unfinished_tool() {
    let mut cfg = config();
    cfg.max_chunks = 1;
    let actual = run(fallback(response(tool("custom_tool_call", false), "completed"), true), cfg);
    assert!(actual.result.unwrap_err().contains("no remaining retry chunk"));
    assert!(actual.events.is_empty());
    assert_eq!((actual.requests.len(), actual.refunds), (3, 1));
}

#[test]
fn repeated_metered_tool_cuts_stop_after_two_repairs() {
    let replies = (0..3).flat_map(|_| fallback(response(tool("function_call", false), "completed"), true)).collect();
    let actual = run(replies, config());
    assert!(actual.result.unwrap_err().contains("two bounded retries"));
    assert!(actual.events.is_empty());
    assert_eq!((actual.requests.len(), actual.refunds), (9, 3));
}

#[test]
fn complete_metered_chunk_preserves_output_usage_and_refund() {
    let item = tool("custom_tool_call", true);
    let expected = response(item.clone(), "completed");
    let actual = run(fallback(expected.clone(), false), config());
    let result = actual.result.unwrap();
    assert_eq!(result.output, vec![item.clone()]);
    assert_eq!(actual.events, vec![(vec![item], false)]);
    assert_eq!((result.chunks, result.metered_chunks, actual.refunds), (1, 1, 1));
    assert_eq!((result.id, result.created_at), ("response-fixture".into(), 9));
    assert_eq!(result.status, "completed");
    assert_eq!(result.usage, expected["usage"]);
    assert_eq!(result.service_tier, expected["service_tier"]);
}

fn text(value: &str) -> Value {
    json!({"type":"message", "id":"message-fixture", "role":"assistant",
        "content":[{"type":"output_text", "text":value}]})
}

#[test]
fn incomplete_metered_text_uses_the_existing_continuation_path() {
    let mut replies = fallback(response(text("first"), "incomplete"), true);
    replies.push(Reply { streamed:false, body:Some(response(text("second"), "completed")), at_budget:false });
    let actual = run(replies, config());
    let result = actual.result.unwrap();
    assert_eq!(result.status, "completed");
    assert_eq!((result.chunks, result.metered_chunks, actual.refunds), (2, 1, 1));
    assert_eq!(actual.events, vec![(vec![text("first")], true), (vec![text("second")], false)]);
    assert!(actual.requests[3]["input"].as_array().unwrap().contains(&text("first")));
    assert_eq!(result.output[0]["content"][0]["text"], "firstsecond");
    assert_eq!(result.output_tokens, actual.requests[2]["max_output_tokens"].as_u64().unwrap() + 7);
}

#[test]
fn final_incomplete_metered_text_preserves_status_and_reason() {
    let mut cfg = config();
    cfg.max_chunks = 1;
    let actual = run(fallback(response(text("partial"), "incomplete"), true), cfg);
    let result = actual.result.unwrap();
    assert_eq!(result.status, "incomplete");
    assert_eq!(result.incomplete_details, Some(json!({"reason":"max_output_tokens"})));
    assert_eq!((result.chunks, result.metered_chunks, actual.refunds), (1, 1, 1));
    assert_eq!(actual.events, vec![(vec![text("partial")], true)]);
}

#[test]
fn disabled_metered_fallback_never_sends_a_stream() {
    let mut cfg = config();
    cfg.metered_fallback = false;
    let replies = fallback(response(tool("function_call", true), "completed"), false)
        .into_iter().take(2).collect();
    let actual = run(replies, cfg);
    assert!(actual.result.unwrap_err().contains("fallback off"));
    assert!(actual.events.is_empty());
    assert_eq!((actual.requests.len(), actual.refunds), (2, 0));
}

#[test]
fn malformed_metered_completion_never_emits_items() {
    let mut invalid_output = response(text("fixture-not-for-output"), "completed");
    invalid_output["output"] = json!({"fixture":"invalid"});
    let mut invalid_usage = response(text("fixture-not-for-output"), "completed");
    invalid_usage["usage"] = json!("fixture-invalid-usage");
    for body in [Value::Null, invalid_output, invalid_usage,
        response(text("fixture-not-for-output"), "failed")] {
        let actual = run(fallback(body, false), config());
        assert!(actual.result.is_err());
        assert!(actual.events.is_empty());
        assert_eq!((actual.requests.len(), actual.refunds), (3, 1));
    }
}

#[test]
fn malformed_nonstream_completion_never_emits_items() {
    let mut invalid_output = response(text("fixture-not-for-output"), "completed");
    invalid_output["output"] = json!({"fixture":"invalid"});
    let mut invalid_usage = response(text("fixture-not-for-output"), "completed");
    invalid_usage["usage"] = json!("fixture-invalid-usage");
    for body in [Value::Null, invalid_output, invalid_usage,
        response(text("fixture-not-for-output"), "failed")] {
        let actual = run(vec![Reply { streamed:false, body:Some(body), at_budget:false }], config());
        assert!(actual.result.is_err());
        assert!(actual.events.is_empty());
        assert_eq!((actual.requests.len(), actual.refunds), (1, 0));
    }
}

fn usage_response(value: &str, status: &str) -> Value {
    let mut body = response(text(value), status);
    body["usage"]["total_tokens"] = json!(18);
    body["usage"]["input_tokens_details"] = json!({"cached_tokens":3});
    body["usage"]["output_tokens_details"] = json!({"reasoning_tokens":2});
    body
}

fn two_chunk_usage() -> Run {
    run(vec![
        Reply { streamed:false, at_budget:false,
            body:Some(usage_response("first", "incomplete")) },
        Reply { streamed:false, at_budget:false,
            body:Some(usage_response("second", "completed")) },
    ], config())
}

#[test]
fn responses_json_total_matches_accumulated_chunk_counters() {
    let actual = two_chunk_usage();
    assert_eq!(actual.requests.len(), 2);
    let result = actual.result.unwrap();
    assert_eq!((result.input_tokens, result.output_tokens, result.chunks), (11, 14, 2));
    let full = framer_core::convert::responses_json(&result, "fixture-model", "local-response");
    assert_eq!(full["usage"]["input_tokens"], 11);
    assert_eq!(full["usage"]["output_tokens"], 14);
    assert_eq!(full["usage"]["total_tokens"], 25);
    assert_eq!(full["usage"]["input_tokens_details"], json!({"cached_tokens":3}));
    assert_eq!(full["usage"]["output_tokens_details"], json!({"reasoning_tokens":2}));
}

#[test]
fn responses_completed_total_matches_accumulated_chunk_counters() {
    let actual = two_chunk_usage();
    assert_eq!(actual.requests.len(), 2);
    let result = actual.result.unwrap();
    let mut composer = framer_core::convert::ResponsesComposer::new("fixture-model", "local-response");
    let mut frames = Vec::new();
    for (items, continues_text) in &actual.events {
        frames.extend(composer.push_items(items, *continues_text));
    }
    frames.extend(composer.finish(&result));
    let completed: Vec<Value> = frames.iter().map(|frame| {
        let data = frame.lines().find_map(|line| line.strip_prefix("data: ")).unwrap();
        serde_json::from_str::<Value>(data).unwrap()
    }).filter(|event| event["type"] == "response.completed").collect();
    assert_eq!(completed.len(), 1);
    let usage = &completed[0]["response"]["usage"];
    assert_eq!(usage["input_tokens"], 11);
    assert_eq!(usage["output_tokens"], 14);
    assert_eq!(usage["total_tokens"], 25);
    assert_eq!(usage["input_tokens_details"], json!({"cached_tokens":3}));
    assert_eq!(usage["output_tokens_details"], json!({"reasoning_tokens":2}));
}

#[test]
fn single_chunk_responses_usage_preserves_total_and_details() {
    let expected = usage_response("only", "completed");
    let actual = run(vec![Reply { streamed:false, at_budget:false,
        body:Some(expected.clone()) }], config());
    assert_eq!(actual.requests.len(), 1);
    let result = actual.result.unwrap();
    assert_eq!((result.input_tokens, result.output_tokens, result.chunks), (11, 7, 1));
    let full = framer_core::convert::responses_json(&result, "fixture-model", "local-response");
    assert_eq!(full["usage"], expected["usage"]);
}

#[test]
fn zero_chunk_limit_cannot_fabricate_a_completed_response() {
    for metered in [false, true] {
        for total in [None, Some(16), Some(4096)] {
            let mut cfg = config();
            cfg.max_chunks = 0;
            cfg.max_total_tokens = total;
            cfg.metered_fallback = metered;
            let actual = run(Vec::new(), cfg);
            assert_eq!(actual.result.unwrap_err(),
                "Responses chain max_chunks must be greater than zero");
            assert!(actual.requests.is_empty());
            assert!(actual.events.is_empty());
            assert_eq!(actual.refunds, 0);
        }
    }
}

#[test]
fn empty_nonstream_completion_remains_valid() {
    let actual = run(vec![Reply { streamed:false, at_budget:false,
        body:Some(json!({"status":"completed", "output":[]})) }], config());
    let result = actual.result.unwrap();
    assert!(result.output.is_empty());
    assert_eq!(result.status, "completed");
    assert_eq!((result.chunks, result.metered_chunks), (1, 0));
    assert_eq!((actual.requests.len(), actual.refunds), (1, 0));
}

fn budget_reply(content: &str, status: &str, output_tokens: u64) -> Reply {
    let mut body = response(text(content), status);
    body["usage"]["output_tokens"] = json!(output_tokens);
    Reply { streamed:false, body:Some(body), at_budget:false }
}

#[test]
fn below_minimum_total_budget_is_refused_before_dispatch() {
    for cap in [0, 1, 15] {
        let mut cfg = config();
        cfg.max_total_tokens = Some(cap);
        let actual = run(vec![], cfg);
        assert_eq!(actual.result.err().as_deref(),
            Some("Responses output budget must be at least 16 tokens"));
        assert!(actual.requests.is_empty());
        assert!(actual.events.is_empty());
        assert_eq!(actual.refunds, 0);
    }
}

#[test]
fn subminimum_remaining_budget_finishes_without_an_extra_request() {
    for cap in [17, 31] {
        let mut cfg = config();
        cfg.max_total_tokens = Some(cap);
        let actual = run(vec![budget_reply("partial", "incomplete", 16)], cfg);
        let result = actual.result.unwrap();
        assert_eq!(actual.requests.len(), 1);
        assert_eq!(actual.requests[0]["max_output_tokens"], cap);
        assert_eq!(result.output, vec![text("partial")]);
        assert_eq!(result.status, "incomplete");
        assert_eq!(result.incomplete_details, Some(json!({"reason":"max_output_tokens"})));
        assert_eq!((result.output_tokens, result.chunks, actual.refunds), (16, 1, 0));
    }
}

#[test]
fn minimum_remaining_budget_still_allows_continuation() {
    let mut cfg = config();
    cfg.max_total_tokens = Some(32);
    let actual = run(vec![budget_reply("partial", "incomplete", 16),
        budget_reply(" tail", "completed", 16)], cfg);
    let result = actual.result.unwrap();
    assert_eq!(actual.requests.len(), 2);
    assert_eq!(actual.requests[0]["max_output_tokens"], 32);
    assert_eq!(actual.requests[1]["max_output_tokens"], 16);
    assert_eq!(result.status, "completed");
    assert_eq!((result.output_tokens, result.chunks, actual.refunds), (32, 2, 0));
}
