use super::*;
use async_trait::async_trait;
use framer_core::ids::FramerUserId;
use framer_core::port::CastleToken;
use std::collections::VecDeque;

#[test]
fn distinct_call_ids_remain_valid_across_published_chunks() {
    let prior = vec![tool("function_call", true), text("first")];
    let mut first = response(Value::Null, "incomplete");
    first["output"] = json!(prior);
    let mut later = tool("custom_tool_call", true);
    later["call_id"] = json!("next-call");
    let actual = run_with_failure(vec![
        Reply { streamed:false, body:Some(first), at_budget:false },
        Reply { streamed:false, body:Some(response(later.clone(), "completed")), at_budget:false },
    ], config(), Some((usize::MAX, "fixture transport connection closed")));
    let mut expected = prior.clone();
    expected.push(later.clone());
    assert_eq!(actual.result.unwrap().output, expected);
    assert_eq!(actual.events, vec![(prior, true), (vec![later], false)]);
    assert_eq!(actual.requests.len(), 2);
}

#[test]
fn discarded_truncated_calls_do_not_reserve_call_ids() {
    for metered in [false, true] {
        let good = tool("function_call", true);
        let mut cut = tool("function_call", false);
        cut["status"] = json!("in_progress");
        let mut first = response(Value::Null, "incomplete");
        first["output"] = json!([good.clone(), cut]);
        let mut replies = tool_body_replies(first, metered);
        replies.push(Reply { streamed:false, at_budget:false,
            body:Some(response(good.clone(), "completed")) });
        let actual = run_with_failure(replies, config(),
            Some((if metered { 1 } else { usize::MAX }, "fixture transport connection closed")));
        assert_eq!(actual.result.unwrap().output, vec![good.clone()]);
        assert_eq!(actual.events, vec![(vec![good], false)]);
        assert_eq!(actual.requests.len(), if metered { 4 } else { 2 });
        assert_eq!(actual.refunds, 0);
    }
}

#[test]
fn distinct_call_ids_preserve_parallel_calls_and_opaque_item_ids() {
    for metered in [false, true] {
        let first = tool("function_call", true);
        let mut second = tool("custom_tool_call", true);
        second["call_id"] = json!("second-call");
        second.as_object_mut().unwrap().remove("status");
        let opaque = json!({"type":"future_tool_call", "call_id":"call-fixture"});
        let expected = vec![first, opaque, second];
        let mut body = response(Value::Null, "completed");
        body["output"] = json!(expected);
        let actual = run_tool_replies(tool_body_replies(body, metered), metered);
        assert_eq!(actual.result.unwrap().output, expected);
        assert_eq!(actual.events, vec![(expected, false)]);
        assert_eq!(actual.requests.len(), if metered { 3 } else { 1 });
    }
}

#[test]
fn duplicate_tool_call_ids_cannot_republish_a_prior_call() {
    for metered in [false, true] {
        let prior = vec![tool("function_call", true), text("first")];
        let mut first = response(Value::Null, "incomplete");
        first["output"] = json!(prior);
        let mut later = tool("custom_tool_call", true);
        later["name"] = json!("different_operation");
        let mut replies = vec![Reply { streamed:false, body:Some(first), at_budget:false }];
        replies.extend(tool_body_replies(response(later, "completed"), metered));
        // The nonmetered case uses an unselected failure index to pause time.
        let failure_index = if metered { 2 } else { usize::MAX };
        let actual = run_with_failure(replies, config(),
            Some((failure_index, "fixture transport connection closed")));
        assert_eq!(actual.result.unwrap_err(), "response chunk has duplicate tool call id");
        assert_eq!(actual.events, vec![(prior, true)]);
        assert_eq!(actual.requests.len(), if metered { 4 } else { 2 });
        assert_eq!(actual.refunds, 0);
    }
}

#[test]
fn duplicate_tool_call_ids_withhold_the_current_chunk() {
    for metered in [false, true] {
        for (first, second) in [("function_call", "function_call"),
            ("custom_tool_call", "custom_tool_call"), ("function_call", "custom_tool_call")] {
            let mut later = tool(second, true);
            later["name"] = json!("different_operation");
            later["id"] = json!("different_item_id");
            let mut body = response(Value::Null, "completed");
            body["output"] = json!([text("withheld"), tool(first, true), later]);
            let actual = run_tool_replies(tool_body_replies(body, metered), metered);
            assert_eq!(actual.result.unwrap_err(), "response chunk has duplicate tool call id");
            assert!(actual.events.is_empty());
            assert_eq!(actual.requests.len(), if metered { 3 } else { 1 });
            assert_eq!(actual.refunds, 0);
        }
    }
}

#[test]
fn invalid_tool_fields_precede_earlier_duplicate_ids() {
    for metered in [false, true] {
        let mut invalid = tool("function_call", true);
        invalid["arguments"] = Value::Null;
        let mut body = response(Value::Null, "completed");
        body["output"] = json!([
            tool("function_call", true), tool("custom_tool_call", true), invalid
        ]);
        let actual = run_tool_replies(tool_body_replies(body, metered), metered);
        assert_eq!(actual.result.unwrap_err(), "response chunk has invalid tool call fields");
        assert!(actual.events.is_empty());
        assert_eq!(actual.requests.len(), if metered { 3 } else { 1 });
    }
}

#[test]
fn exact_call_ids_and_matching_result_references_remain_verbatim() {
    for metered in [false, true] {
        let mut expected = Vec::new();
        for (kind, id) in [("function_call", "call-fixture"),
            ("custom_tool_call", " call-fixture"), ("function_call", "call-fixture "),
            ("custom_tool_call", "Call-Fixture"), ("function_call", "é"),
            ("custom_tool_call", "e\u{301}")] {
            let mut call = tool(kind, true);
            call["call_id"] = json!(id);
            call.as_object_mut().unwrap().remove("id");
            let result_kind = format!("{kind}_output");
            expected.push(call);
            expected.push(json!({"type":result_kind, "call_id":id, "output":"fixture"}));
        }
        let mut body = response(Value::Null, "completed");
        body["output"] = json!(expected);
        let actual = run_tool_replies(tool_body_replies(body, metered), metered);
        assert_eq!(actual.result.unwrap().output, expected);
        assert_eq!(actual.events, vec![(expected, false)]);
        assert_eq!(actual.requests.len(), if metered { 3 } else { 1 });
    }
}

#[test]
fn repeated_sse_call_references_do_not_duplicate_accepted_output() {
    let call = tool("function_call", true);
    let prefix = concat!(
        "event: response.output_item.added\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"call_id\":\"call-fixture\"}}\n\n",
        "event: response.output_item.done\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"call_id\":\"call-fixture\"}}\n\n",
    );
    let actual = run_with_frame_prefix(fallback(response(call.clone(), "completed"), false),
        config(), Some((1, "fixture transport connection closed")), "response.completed", prefix);
    assert_eq!(actual.result.unwrap().output, vec![call.clone()]);
    assert_eq!(actual.events, vec![(vec![call], false)]);
    assert_eq!((actual.requests.len(), actual.refunds), (3, 0));
}

#[test]
fn invalid_message_roles_withhold_the_entire_current_chunk() {
    for metered in [false, true] {
        for role in [json!("user"), json!("system"), json!("developer"), json!("tool"),
            json!(""), Value::Null, json!(false), json!(7), json!([]), json!({})] {
            let mut bad = text("unpublished");
            bad["role"] = role;
            let mut body = response(text("also withheld"), "completed");
            body["output"].as_array_mut().unwrap().push(bad);
            let actual = run_tool_replies(tool_body_replies(body, metered), metered);
            let prefix = if metered { "streamed chunk" } else { "response chunk" };
            assert_eq!(actual.result.unwrap_err(), format!("{prefix} has invalid message role"));
            assert!(actual.events.is_empty());
            assert_eq!((actual.requests.len(), actual.refunds),
                (if metered { 3 } else { 1 }, 0));
        }
    }
}

#[test]
fn invalid_later_message_role_preserves_published_output() {
    for metered in [false, true] {
        let mut bad = text("unpublished");
        bad["role"] = json!("user");
        let mut replies = vec![counted_reply(11, 7, "incomplete", "first")];
        replies.extend(tool_body_replies(response(bad, "completed"), metered));
        let actual = run_with_failure(replies, config(),
            Some((if metered { 2 } else { usize::MAX }, "fixture transport connection closed")));
        let prefix = if metered { "streamed chunk" } else { "response chunk" };
        assert_eq!(actual.result.unwrap_err(), format!("{prefix} has invalid message role"));
        assert_eq!(actual.events, vec![(vec![text("first")], true)]);
        assert_eq!((actual.requests.len(), actual.refunds),
            (if metered { 4 } else { 2 }, 0));
    }
}

#[test]
fn assistant_and_absent_roles_preserve_accepted_chunk_metadata() {
    for metered in [false, true] {
        for role in [None, Some(json!("assistant"))] {
            let mut message = text("answer");
            message.as_object_mut().unwrap().remove("role");
            if let Some(role) = role { message["role"] = role; }
            message["extension"] = json!({"keep":true});
            let expected = vec![message, json!({"type":"future_item", "role":"user"})];
            let mut body = response(Value::Null, "completed");
            body["output"] = json!(expected);
            let actual = run_tool_replies(tool_body_replies(body, metered), metered);
            assert_eq!(actual.result.unwrap().output, expected);
            assert_eq!(actual.events, vec![(expected, false)]);
            assert_eq!(actual.requests.len(), if metered { 3 } else { 1 });
        }
    }
}

#[test]
fn response_error_withholds_the_entire_current_chunk() {
    for metered in [false, true] {
        for error in [json!({"code":"server_error", "message":"fixture diagnostic"}),
            json!({}), json!("fixture diagnostic"), json!(false), json!(7), json!([])] {
            let mut body = response(text("withheld"), "completed");
            let has_cause = error.is_string() || error["message"].is_string();
            body["error"] = error;
            let actual = run_tool_replies(tool_body_replies(body, metered), metered);
            let prefix = if metered { "streamed chunk" } else { "response chunk" };
            let expected = if has_cause {
                format!("{prefix} contains an upstream error: fixture diagnostic")
            } else { format!("{prefix} has non-null response error") };
            assert_eq!(actual.result.unwrap_err(), expected);
            assert!(actual.events.is_empty());
            assert_eq!((actual.requests.len(), actual.refunds),
                (if metered { 3 } else { 1 }, 0));
        }
    }
}

#[test]
fn later_response_error_preserves_previously_published_output() {
    for metered in [false, true] {
        let mut body = response(text("withheld"), "completed");
        body["error"] = json!({"code":"server_error", "message":"fixture diagnostic"});
        let mut replies = vec![counted_reply(11, 7, "incomplete", "first")];
        replies.extend(tool_body_replies(body, metered));
        let actual = run_with_failure(replies, config(),
            Some((if metered { 2 } else { usize::MAX }, "fixture transport connection closed")));
        let prefix = if metered { "streamed chunk" } else { "response chunk" };
        assert_eq!(actual.result.unwrap_err(),
            format!("{prefix} contains an upstream error: fixture diagnostic"));
        assert_eq!(actual.events, vec![(vec![text("first")], true)]);
        assert_eq!((actual.requests.len(), actual.refunds),
            (if metered { 4 } else { 2 }, 0));
    }
}

#[test]
fn absent_or_null_response_error_preserves_accepted_chunk_output() {
    for metered in [false, true] {
        for error in [None, Some(Value::Null)] {
            let expected = vec![text("answer"), json!({"type":"future_item", "error":{"opaque":true}})];
            let mut body = response(Value::Null, "completed");
            body["output"] = json!(expected);
            if let Some(error) = error { body["error"] = error; }
            let actual = run_tool_replies(tool_body_replies(body, metered), metered);
            assert_eq!(actual.result.unwrap().output, expected);
            assert_eq!(actual.events, vec![(expected, false)]);
            assert_eq!((actual.requests.len(), actual.refunds),
                (if metered { 3 } else { 1 }, 0));
        }
    }
}

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
    failure: Option<(usize, &'static str)>,
    terminal_event: &'static str,
    frame_prefix: &'static str,
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
        if let Some((attempt, error)) = self.failure {
            if self.requests.lock().unwrap().len() == attempt {
                return Err(error.into());
            }
        }
        let Some(mut response) = reply.body else {
            return Err("fixture transport connection closed".into());
        };
        if reply.at_budget {
            response["usage"]["output_tokens"] = body["request"]["max_output_tokens"].clone();
        }
        let text = if reply.streamed {
            format!("{}data: {}\n\n", self.frame_prefix,
                json!({"type":self.terminal_event, "response":response}))
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
    run_with_failure(replies, cfg, None)
}

fn run_with_failure(replies: Vec<Reply>, cfg: ChainConfig,
    failure: Option<(usize, &'static str)>) -> Run {
    run_with_terminal_event(replies, cfg, failure, "response.completed")
}

fn run_with_terminal_event(replies: Vec<Reply>, cfg: ChainConfig,
    failure: Option<(usize, &'static str)>, terminal_event: &'static str) -> Run {
    run_with_frame_prefix(replies, cfg, failure, terminal_event, "")
}

fn run_with_frame_prefix(replies: Vec<Reply>, cfg: ChainConfig,
    failure: Option<(usize, &'static str)>, terminal_event: &'static str,
    frame_prefix: &'static str) -> Run {
    tokio::runtime::Builder::new_current_thread().enable_all()
        .start_paused(failure.is_some()).build().unwrap().block_on(async {
        let account = AccountId(uuid::Uuid::from_u128(1));
        let session = Session { active_user:FramerUserId("fixture-user".into()), cookie_header:"fixture-only".into() };
        let cache = Mutex::new(vec![(account, AiToken::for_session(&session, "fixture-only".into(), i64::MAX))]);
        let browser = Fixture { replies:Mutex::new(replies.into()), requests:Mutex::new(Vec::new()),
            refunds:Mutex::new(0), failure, terminal_event, frame_prefix };
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
    assert_eq!((result.chunks, result.metered_chunks, actual.refunds), (2, 1, 0));
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
    assert_eq!((actual.requests.len(), actual.refunds), (3, 0));
}

#[test]
fn repeated_metered_tool_cuts_stop_after_two_repairs() {
    let replies = (0..3).flat_map(|_| fallback(response(tool("function_call", false), "completed"), true)).collect();
    let actual = run(replies, config());
    assert!(actual.result.unwrap_err().contains("two bounded retries"));
    assert!(actual.events.is_empty());
    assert_eq!((actual.requests.len(), actual.refunds), (9, 0));
}

#[test]
fn complete_metered_chunk_preserves_output_usage_without_refund() {
    let item = tool("custom_tool_call", true);
    let expected = response(item.clone(), "completed");
    let actual = run(fallback(expected.clone(), false), config());
    let result = actual.result.unwrap();
    assert_eq!(result.output, vec![item.clone()]);
    assert_eq!(actual.events, vec![(vec![item], false)]);
    assert_eq!((result.chunks, result.metered_chunks, actual.refunds), (1, 1, 0));
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
    assert_eq!((result.chunks, result.metered_chunks, actual.refunds), (2, 1, 0));
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
    assert_eq!((result.chunks, result.metered_chunks, actual.refunds), (1, 1, 0));
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

fn refuses_after_transport_retry(error: &'static str) {
    for metered in [true, false] {
        let mut cfg = config();
        cfg.metered_fallback = metered;
        let replies = fallback(response(text("unexpected"), "completed"), false)
            .into_iter().take(2).collect();
        let actual = run_with_failure(replies, cfg, Some((2, error)));
        assert_eq!(actual.result.unwrap_err(), error);
        assert_eq!((actual.requests.len(), actual.refunds), (2, 0));
        assert!(actual.requests.iter().all(|request| request["stream"] == false));
        assert!(actual.events.is_empty());
    }
}

#[test]
fn retry_risk_refusal_never_enters_metered_fallback() {
    refuses_after_transport_retry("risk-refused");
}

#[test]
fn retry_provider_refusal_never_enters_metered_fallback() {
    refuses_after_transport_retry("upstream 403 on /ai/v3/responses/: provider refused");
}

#[test]
fn retry_credit_exhaustion_never_enters_metered_fallback() {
    refuses_after_transport_retry("credits-exhausted");
}

#[test]
fn retry_client_errors_never_enter_metered_fallback() {
    for error in ["retryable 400 on /ai/v3/responses/: bad request",
        "retryable 404 on /ai/v3/responses/: missing route",
        "retryable 422 on /ai/v3/responses/: invalid input"] {
        refuses_after_transport_retry(error);
    }
}

#[test]
fn retry_rate_limit_never_enters_metered_fallback() {
    refuses_after_transport_retry("retryable 429 on /ai/v3/responses/: busy");
}

#[test]
fn retry_local_cooldown_retains_existing_no_fallback_behavior() {
    refuses_after_transport_retry("account-cooling:30");
}

#[test]
fn transport_retry_policy_preserves_security_refusals_at_every_elapsed_time() {
    assert!(!allows_transport_retry("risk-refused", 8.0));
    assert!(!allows_transport_retry("risk-refused", 8.1));
    assert!(allows_transport_retry("fixture transport connection closed", 0.0));
    assert!(allows_transport_retry("retryable 503 on /ai/v3/responses/: unavailable", 1.0));
    for error in ["unauthorized", "credits-exhausted", "upstream 403 on fixture", "retryable 429 on fixture"] {
        assert!(!allows_transport_retry(error, 0.0));
        assert!(!allows_transport_retry(error, 90.0));
    }
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
        assert_eq!((actual.requests.len(), actual.refunds), (3, 0));
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

fn counted_reply(input: u64, output: u64, status: &str, value: &str) -> Reply {
    let mut body = response(text(value), status);
    body["usage"]["input_tokens"] = json!(input);
    body["usage"]["output_tokens"] = json!(output);
    Reply { streamed:false, at_budget:false, body:Some(body) }
}

#[test]
fn overflowing_single_chunk_total_is_refused_before_emission() {
    let actual = run(vec![counted_reply(u64::MAX, 1, "completed", "private-fixture")], config());
    assert_eq!(actual.result.unwrap_err(), "Responses total usage overflow");
    assert!(actual.events.is_empty());
    assert_eq!(actual.requests.len(), 1);
}

#[test]
fn overflowing_metered_total_is_refused_before_emission() {
    let body = counted_reply(u64::MAX, 1, "completed", "private-fixture").body.unwrap();
    let actual = run(fallback(body, false), config());
    assert_eq!(actual.result.unwrap_err(), "Responses total usage overflow");
    assert!(actual.events.is_empty());
    assert_eq!(actual.requests.len(), 3);
}

#[test]
fn accumulating_output_overflow_withholds_the_offending_chunk() {
    let mut cfg = config();
    cfg.max_total_tokens = None;
    cfg.max_chunks = 2;
    let actual = run(vec![
        counted_reply(0, u64::MAX, "incomplete", "first"),
        counted_reply(0, 1, "completed", "private-fixture"),
    ], cfg);
    assert_eq!(actual.result.unwrap_err(), "Responses output usage overflow");
    assert_eq!(actual.events, vec![(vec![text("first")], true)]);
    assert_eq!(actual.requests.len(), 2);
}

#[test]
fn larger_input_count_cannot_overflow_accumulated_total() {
    let mut cfg = config();
    cfg.max_total_tokens = None;
    cfg.max_chunks = 2;
    let actual = run(vec![
        counted_reply(2, u64::MAX - 8, "incomplete", "first"),
        counted_reply(6, 3, "completed", "private-fixture"),
    ], cfg);
    assert_eq!(actual.result.unwrap_err(), "Responses total usage overflow");
    assert_eq!(actual.events, vec![(vec![text("first")], true)]);
    assert_eq!(actual.requests.len(), 2);
}

#[test]
fn maximum_representable_total_remains_valid() {
    let actual = run(vec![counted_reply(u64::MAX - 7, 7, "completed", "answer")], config());
    let result = actual.result.unwrap();
    assert_eq!((result.input_tokens, result.output_tokens), (u64::MAX - 7, 7));
    let full = framer_core::convert::responses_json(&result, "fixture-model", "local-response");
    assert_eq!(full["usage"]["total_tokens"], json!(u64::MAX));
    assert_eq!(actual.events, vec![(vec![text("answer")], false)]);
    assert_eq!(actual.requests.len(), 1);
}

#[test]
fn maximum_representable_accumulated_output_remains_valid() {
    let mut cfg = config();
    cfg.max_total_tokens = None;
    cfg.max_chunks = 2;
    let actual = run(vec![
        counted_reply(0, u64::MAX - 1, "incomplete", "first"),
        counted_reply(0, 1, "completed", "second"),
    ], cfg);
    let result = actual.result.unwrap();
    assert_eq!((result.input_tokens, result.output_tokens, result.chunks), (0, u64::MAX, 2));
    let full = framer_core::convert::responses_json(&result, "fixture-model", "local-response");
    assert_eq!(full["usage"]["total_tokens"], json!(u64::MAX));
    assert_eq!(actual.events, vec![(vec![text("first")], true), (vec![text("second")], false)]);
    assert_eq!(actual.requests.len(), 2);
}

fn tool_body_replies(body: Value, metered: bool) -> Vec<Reply> {
    if metered { fallback(body, false) }
    else { vec![Reply { streamed:false, body:Some(body), at_budget:false }] }
}

fn run_tool_replies(replies: Vec<Reply>, metered: bool) -> Run {
    // Metered fixtures inject the first transport failure under paused time.
    run_with_failure(replies, config(),
        metered.then_some((1, "fixture transport connection closed")))
}

fn assert_tool_refused(item: Value, metered: bool) {
    let actual = run_tool_replies(tool_body_replies(response(item, "completed"), metered), metered);
    assert_eq!(actual.result.unwrap_err(), "response chunk has invalid tool call fields");
    assert!(actual.events.is_empty());
    assert_eq!(actual.requests.len(), if metered { 3 } else { 1 });
    assert_eq!(actual.refunds, 0);
}

fn reject_tool_identity(metered: bool) {
    for kind in ["function_call", "custom_tool_call"] {
        for field in ["call_id", "name"] {
            for bad in [None, Some(Value::Null), Some(json!(7)), Some(json!(""))] {
                let mut item = tool(kind, true);
                if let Some(bad) = bad { item[field] = bad; }
                else { item.as_object_mut().unwrap().remove(field); }
                assert_tool_refused(item, metered);
            }
        }
    }
}

#[test]
fn nonstream_tool_calls_require_identity_fields() { reject_tool_identity(false); }

#[test]
fn metered_tool_calls_require_identity_fields() { reject_tool_identity(true); }

fn reject_tool_input(metered: bool) {
    for (kind, field) in [("function_call", "arguments"), ("custom_tool_call", "input")] {
        for bad in [None, Some(Value::Null), Some(json!({"private-fixture":1})), Some(json!([]))] {
            let mut item = tool(kind, true);
            if let Some(bad) = bad { item[field] = bad; }
            else { item.as_object_mut().unwrap().remove(field); }
            assert_tool_refused(item, metered);
        }
    }
}

#[test]
fn nonstream_tool_calls_require_string_input() { reject_tool_input(false); }

#[test]
fn metered_tool_calls_require_string_input() { reject_tool_input(true); }

#[test]
fn complete_tools_preserve_optional_metadata_and_empty_input() {
    for metered in [false, true] {
        for (kind, field) in [("function_call", "arguments"), ("custom_tool_call", "input")] {
            let mut item = tool(kind, true);
            item[field] = json!("");
            for supplied_metadata in [false, true] {
                if supplied_metadata {
                    item["id"] = json!("upstream-item");
                    item["status"] = json!("completed");
                    item["namespace"] = json!("fixture-namespace");
                    item["extension"] = json!({"preserve":[1,2]});
                } else {
                    item.as_object_mut().unwrap().remove("id");
                    item.as_object_mut().unwrap().remove("status");
                }
                let actual = run_tool_replies(
                    tool_body_replies(response(item.clone(), "completed"), metered), metered);
                let result = actual.result.unwrap();
                assert_eq!(result.output, vec![item.clone()]);
                assert_eq!(actual.events, vec![(vec![item.clone()], false)]);
                assert_eq!(result.metered_chunks, u32::from(metered));
            }
        }
    }
}

#[test]
fn custom_tool_text_remains_opaque() {
    for metered in [false, true] {
        let mut item = tool("custom_tool_call", true);
        item["input"] = json!("not JSON: {\n  exact bytes 雪\n");
        let actual = run_tool_replies(
            tool_body_replies(response(item.clone(), "completed"), metered), metered);
        assert_eq!(actual.result.unwrap().output, vec![item.clone()]);
        assert_eq!(actual.events, vec![(vec![item], false)]);
    }
}

#[test]
fn unrelated_output_items_remain_verbatim() {
    for metered in [false, true] {
        let items = json!([text("answer"),
            {"type":"reasoning", "id":"reason", "summary":[]},
            {"type":"future_tool_call", "extension":{"opaque":true}}]);
        let mut body = response(Value::Null, "completed");
        body["output"] = items.clone();
        let actual = run_tool_replies(tool_body_replies(body, metered), metered);
        let expected = items.as_array().unwrap().clone();
        assert_eq!(actual.result.unwrap().output, expected);
        assert_eq!(actual.events, vec![(expected, false)]);
    }
}

#[test]
fn tool_field_validation_preserves_truncation_retry() {
    for metered in [false, true] {
        for (kind, field) in [("function_call", "arguments"), ("custom_tool_call", "input")] {
            let good = tool(kind, true);
            let mut cut = good.clone();
            cut.as_object_mut().unwrap().remove(field);
            cut.as_object_mut().unwrap().remove("call_id");
            let mut replies = tool_body_replies(response(cut, "incomplete"), metered);
            replies.push(Reply { streamed:false, at_budget:false,
                body:Some(response(good.clone(), "completed")) });
            let actual = run_tool_replies(replies, metered);
            assert_eq!(actual.result.unwrap().output, vec![good.clone()]);
            assert_eq!(actual.events, vec![(vec![good], false)]);
            assert_eq!(actual.requests.len(), if metered { 4 } else { 2 });
        }
    }
}

#[test]
fn invalid_later_tool_preserves_previously_emitted_text() {
    let mut bad = tool("function_call", true);
    bad["name"] = Value::Null;
    let actual = run(vec![counted_reply(11, 7, "incomplete", "first"),
        Reply { streamed:false, at_budget:false, body:Some(response(bad, "completed")) }], config());
    assert_eq!(actual.result.unwrap_err(), "response chunk has invalid tool call fields");
    assert_eq!(actual.events, vec![(vec![text("first")], true)]);
    assert_eq!(actual.requests.len(), 2);
}

#[test]
fn invalid_tool_withholds_entire_current_chunk() {
    for metered in [false, true] {
        let mut bad = tool("function_call", true);
        bad["arguments"] = json!({"private-fixture":"must not appear in errors"});
        let mut body = response(Value::Null, "completed");
        body["output"] = json!([tool("custom_tool_call", true), bad, text("after")]);
        let actual = run_tool_replies(tool_body_replies(body, metered), metered);
        assert_eq!(actual.result.unwrap_err(), "response chunk has invalid tool call fields");
        assert!(actual.events.is_empty());
        assert_eq!(actual.requests.len(), if metered { 3 } else { 1 });
    }
}

fn reject_explicit_tool_status(metered: bool) {
    for kind in ["function_call", "custom_tool_call"] {
        for status in [json!("incomplete"), json!("in_progress"), Value::Null,
            json!(true), json!(0), json!({}), json!([]), json!(""), json!("unknown")] {
            let mut item = tool(kind, true);
            item["status"] = status;
            assert_tool_refused(item, metered);
        }
    }
}

#[test]
fn nonstream_tool_calls_reject_unfinished_or_invalid_status() {
    reject_explicit_tool_status(false);
}

#[test]
fn metered_tool_calls_reject_unfinished_or_invalid_status() {
    reject_explicit_tool_status(true);
}

#[test]
fn parent_incomplete_does_not_invalidate_finished_or_statusless_tools() {
    for metered in [false, true] {
        for kind in ["function_call", "custom_tool_call"] {
            for explicit_status in [false, true] {
                let mut item = tool(kind, true);
                if !explicit_status { item.as_object_mut().unwrap().remove("status"); }
                let mut body = response(item.clone(), "incomplete");
                body["incomplete_details"] = json!({"reason":"content_filter"});
                let actual = run_tool_replies(tool_body_replies(body, metered), metered);
                let result = actual.result.unwrap();
                assert_eq!(result.output, vec![item.clone()]);
                assert_eq!(result.status, "incomplete");
                assert_eq!(result.incomplete_details, Some(json!({"reason":"content_filter"})));
                assert_eq!(actual.events, vec![(vec![item], false)]);
                assert_eq!(actual.requests.len(), if metered { 3 } else { 1 });
            }
        }
    }
}

#[test]
fn truncated_tool_status_is_repaired_before_status_validation() {
    for metered in [false, true] {
        for kind in ["function_call", "custom_tool_call"] {
            for status in [json!("in_progress"), Value::Null] {
                let good = tool(kind, true);
                let mut cut = tool(kind, false);
                cut["status"] = status;
                let mut replies = tool_body_replies(response(cut, "incomplete"), metered);
                replies.push(Reply { streamed:false, at_budget:false,
                    body:Some(response(good.clone(), "completed")) });
                let actual = run_tool_replies(replies, metered);
                assert_eq!(actual.result.unwrap().output, vec![good.clone()]);
                assert_eq!(actual.events, vec![(vec![good], false)]);
                assert_eq!(actual.requests.len(), if metered { 4 } else { 2 });
            }
        }
    }
}

#[test]
fn truncated_invalid_tool_status_keeps_final_chunk_refusal() {
    for metered in [false, true] {
        for kind in ["function_call", "custom_tool_call"] {
            let mut cut = tool(kind, false);
            cut["status"] = json!("incomplete");
            let mut cfg = config();
            cfg.max_chunks = 1;
            let actual = run_with_failure(
                tool_body_replies(response(cut, "incomplete"), metered), cfg,
                metered.then_some((1, "fixture transport connection closed")));
            assert_eq!(actual.result.unwrap_err(),
                "chunk 0: incomplete tool call has no remaining retry chunk");
            assert!(actual.events.is_empty());
            assert_eq!(actual.requests.len(), if metered { 3 } else { 1 });
        }
    }
}

#[test]
fn unrecognized_output_status_remains_opaque() {
    for metered in [false, true] {
        for status in [Value::Null, json!(false), json!("in_progress")] {
            let item = json!({"type":"future_output", "status":status,
                "extension":{"unchanged":true}});
            let actual = run_tool_replies(
                tool_body_replies(response(item.clone(), "completed"), metered), metered);
            assert_eq!(actual.result.unwrap().output, vec![item.clone()]);
            assert_eq!(actual.events, vec![(vec![item], false)]);
        }
    }
}

fn run_explicit_incomplete(replies: Vec<Reply>, cfg: ChainConfig) -> Run {
    run_with_terminal_event(replies, cfg, Some((1, "fixture transport connection closed")),
        "response.incomplete")
}

#[test]
fn explicit_incomplete_metered_text_uses_existing_continuation() {
    let mut replies = fallback(response(text("first"), "incomplete"), true);
    replies.push(Reply { streamed:false, at_budget:false,
        body:Some(response(text("second"), "completed")) });
    let actual = run_explicit_incomplete(replies, config());
    let result = actual.result.unwrap();
    assert_eq!(result.status, "completed");
    assert_eq!((result.chunks, result.metered_chunks, actual.refunds), (2, 1, 0));
    assert_eq!(actual.events, vec![(vec![text("first")], true), (vec![text("second")], false)]);
    assert_eq!(result.output[0]["content"][0]["text"], "firstsecond");
    assert!(actual.requests[3]["input"].as_array().unwrap().contains(&text("first")));
    assert_eq!(actual.requests.len(), 4);
}

#[test]
fn explicit_incomplete_final_text_preserves_partial_status() {
    let mut cfg = config();
    cfg.max_chunks = 1;
    let actual = run_explicit_incomplete(fallback(response(text("partial"), "incomplete"), true), cfg);
    let result = actual.result.unwrap();
    assert_eq!(result.status, "incomplete");
    assert_eq!(result.incomplete_details, Some(json!({"reason":"max_output_tokens"})));
    assert_eq!(result.output, vec![text("partial")]);
    assert_eq!(actual.events, vec![(vec![text("partial")], true)]);
    assert_eq!((actual.requests.len(), actual.refunds), (3, 0));
}

#[test]
fn explicit_incomplete_tool_is_withheld_and_repaired() {
    for kind in ["function_call", "custom_tool_call"] {
        let mut cut = tool(kind, false);
        cut["status"] = json!("in_progress");
        let good = tool(kind, true);
        let mut replies = fallback(response(cut, "incomplete"), true);
        replies.push(Reply { streamed:false, at_budget:false,
            body:Some(response(good.clone(), "completed")) });
        let actual = run_explicit_incomplete(replies, config());
        assert_eq!(actual.result.unwrap().output, vec![good.clone()]);
        assert_eq!(actual.events, vec![(vec![good], false)]);
        assert_eq!(actual.requests.len(), 4);
    }
}

fn reject_invalid_usage_counts(metered: bool) {
    for field in ["input_tokens", "output_tokens"] {
        for bad in [Value::Null, json!("7"), json!(-1), json!(1.5),
            json!(true), json!({}), json!([]), json!(1e100)] {
            let mut body = response(tool("custom_tool_call", true), "completed");
            body["usage"][field] = bad;
            let actual = run_tool_replies(tool_body_replies(body, metered), metered);
            let prefix = if metered { "streamed chunk" } else { "response chunk" };
            assert_eq!(actual.result.unwrap_err(), format!("{prefix} has invalid usage count"));
            assert!(actual.events.is_empty());
            assert_eq!(actual.requests.len(), if metered { 3 } else { 1 });
            assert_eq!(actual.refunds, 0);
        }
    }
}

#[test]
fn nonstream_invalid_usage_cannot_publish_tools() { reject_invalid_usage_counts(false); }

#[test]
fn metered_invalid_usage_cannot_publish_tools() { reject_invalid_usage_counts(true); }

#[test]
fn invalid_usage_cannot_start_an_unaccounted_continuation() {
    let mut body = response(text("partial"), "incomplete");
    body["usage"]["output_tokens"] = json!("16");
    let mut cfg = config();
    cfg.max_total_tokens = Some(16);
    cfg.max_chunks = 2;
    let actual = run_with_failure(fallback(body, false), cfg,
        Some((1, "fixture transport connection closed")));
    assert_eq!(actual.result.unwrap_err(), "streamed chunk has invalid usage count");
    assert!(actual.events.is_empty());
    assert_eq!(actual.requests.len(), 3);
}

#[test]
fn invalid_later_usage_preserves_already_emitted_output() {
    let mut bad = response(text("second"), "completed");
    bad["usage"]["input_tokens"] = json!(-1);
    let actual = run(vec![counted_reply(11, 7, "incomplete", "first"),
        Reply { streamed:false, at_budget:false, body:Some(bad) }], config());
    assert_eq!(actual.result.unwrap_err(), "response chunk has invalid usage count");
    assert_eq!(actual.events, vec![(vec![text("first")], true)]);
    assert_eq!(actual.requests.len(), 2);
}

fn reject_malformed_output_items(metered: bool) {
    for bad in [Value::Null, json!(true), json!(42), json!("private-fixture"),
        json!([]), json!({}), json!({"type":null}), json!({"type":false}),
        json!({"type":42}), json!({"type":[]}), json!({"type":{}}), json!({"type":""})] {
        for status in ["completed", "incomplete"] {
            let mut body = response(text("unpublished"), status);
            body["output"].as_array_mut().unwrap().push(bad.clone());
            let actual = run_tool_replies(tool_body_replies(body, metered), metered);
            let prefix = if metered { "streamed chunk" } else { "response chunk" };
            assert_eq!(actual.result.unwrap_err(), format!("{prefix} has invalid output item"));
            assert!(actual.events.is_empty());
            assert_eq!(actual.requests.len(), if metered { 3 } else { 1 });
            assert_eq!(actual.refunds, 0);
        }
    }
}

#[test]
fn nonstream_malformed_output_items_never_publish() { reject_malformed_output_items(false); }

#[test]
fn metered_malformed_output_items_never_publish() { reject_malformed_output_items(true); }

#[test]
fn malformed_later_output_keeps_only_previously_published_items() {
    let mut bad = response(text("unpublished"), "completed");
    bad["output"].as_array_mut().unwrap().push(json!({}));
    let actual = run(vec![counted_reply(11, 7, "incomplete", "first"),
        Reply { streamed:false, at_budget:false, body:Some(bad) }], config());
    assert_eq!(actual.result.unwrap_err(), "response chunk has invalid output item");
    assert_eq!(actual.events, vec![(vec![text("first")], true)]);
    assert_eq!((actual.requests.len(), actual.refunds), (2, 0));
}

#[test]
fn named_error_fallback_cannot_publish_executable_tools() {
    for prefix in ["event:error\n", "event:ping\nevent:error\n",
        "event:error\n\n", "event:error\r\n"] {
        let actual = run_with_frame_prefix(
            fallback(response(tool("function_call", true), "completed"), false),
            config(), Some((1, "fixture transport connection closed")),
            "response.completed", prefix);
        assert_eq!(actual.result.unwrap_err(), "streamed chunk ended with error: the upstream supplied no further cause");
        assert!(actual.events.is_empty());
        assert_eq!((actual.requests.len(), actual.refunds), (3, 0));
    }
}

#[test]
fn overwritten_error_name_keeps_normal_fallback_output() {
    let item = tool("custom_tool_call", true);
    for prefix in ["event:error\nevent:ping\n", "event:error\nevent\n\n"] {
        let actual = run_with_frame_prefix(
            fallback(response(item.clone(), "completed"), false), config(),
            Some((1, "fixture transport connection closed")), "response.completed", prefix);
        let result = actual.result.unwrap();
        assert_eq!(result.output, vec![item.clone()]);
        assert_eq!(actual.events, vec![(vec![item.clone()], false)]);
        assert_eq!((result.chunks, result.metered_chunks), (1, 1));
        assert_eq!((actual.requests.len(), actual.refunds), (3, 0));
    }
}

#[test]
fn malformed_message_content_never_publishes_in_either_response_mode() {
    for metered in [false, true] {
        for (item, reason) in [
            (json!({"type":"message"}), "invalid message content"),
            (json!({"type":"message", "content":"private-fixture"}), "invalid message content"),
            (json!({"type":"message", "content":[null]}), "invalid message part"),
            (json!({"type":"message", "content":[{"type":"output_text", "text":7}]}), "invalid output text"),
            (json!({"type":"message", "content":[{"type":"refusal", "refusal":false}]}), "invalid refusal"),
        ] {
            for status in ["completed", "incomplete"] {
                let mut body = response(text("unpublished"), status);
                body["output"].as_array_mut().unwrap().push(item.clone());
                let actual = run_tool_replies(tool_body_replies(body, metered), metered);
                let prefix = if metered { "streamed chunk" } else { "response chunk" };
                assert_eq!(actual.result.unwrap_err(), format!("{prefix} has {reason}"));
                assert!(actual.events.is_empty());
                assert_eq!(actual.requests.len(), if metered { 3 } else { 1 });
                assert_eq!(actual.refunds, 0);
            }
        }
    }
}

#[test]
fn invalid_later_message_content_keeps_only_prior_events() {
    let bad = json!({"type":"message", "content":[{"type":"output_text"}]});
    let actual = run(vec![counted_reply(11, 7, "incomplete", "first"),
        Reply { streamed:false, at_budget:false, body:Some(response(bad, "completed")) }], config());
    assert_eq!(actual.result.unwrap_err(), "response chunk has invalid output text");
    assert_eq!(actual.events, vec![(vec![text("first")], true)]);
    assert_eq!((actual.requests.len(), actual.refunds), (2, 0));
}

#[test]
fn mixed_message_content_retains_refusal_extensions_and_empty_values() {
    let item = json!({"type":"message", "content":[
        {"type":"output_text", "text":"", "annotations":[{"fixture":true}]},
        {"type":"refusal", "refusal":"no", "extra":[1,2]},
        {"type":"future_part", "opaque":{"kept":true}}]});
    for metered in [false, true] {
        let actual = run_tool_replies(
            tool_body_replies(response(item.clone(), "completed"), metered), metered);
        let result = actual.result.unwrap();
        assert_eq!(result.output, vec![item.clone()]);
        assert_eq!(actual.events, vec![(vec![item.clone()], false)]);
        assert_eq!(result.usage, json!({"input_tokens":11, "output_tokens":7}));
        assert_eq!(actual.requests.len(), if metered { 3 } else { 1 });
    }
}

#[test]
fn named_terminal_failures_in_fallback_do_not_publish() {
    for (prefix, expected) in [
        ("event: response.failed\n", "streamed chunk ended with response.failed: the upstream supplied no further cause"),
        ("event: response.incomplete\n", "streamed chunk has invalid named terminal event"),
        ("event: response.completed\ndata: not-json\n\n", "streamed chunk has invalid named terminal event"),
    ] {
        let actual = run_with_frame_prefix(
            fallback(response(tool("function_call", true), "completed"), false), config(),
            Some((1, "fixture transport connection closed")), "response.completed", prefix);
        assert_eq!(actual.result.unwrap_err(), expected);
        assert!(actual.events.is_empty());
        assert_eq!((actual.requests.len(), actual.refunds), (3, 0));
    }
}

#[test]
fn matching_named_fallback_keeps_completed_output() {
    let item = text("answer");
    let actual = run_with_frame_prefix(fallback(response(item.clone(), "completed"), false),
        config(), Some((1, "fixture transport connection closed")), "response.completed",
        "event: response.failed\nevent: response.completed\n");
    let result = actual.result.unwrap();
    assert_eq!(result.output, vec![item.clone()]);
    assert_eq!(result.status, "completed");
    assert_eq!(actual.events, vec![(vec![item], false)]);
    assert_eq!((actual.requests.len(), actual.refunds), (3, 0));
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

#[test]
fn malformed_earlier_completion_withholds_later_fallback_tools() {
    for (prefix, reason) in [
        ("data: {\"type\":\"response.completed\",\"response\":null}\n\n", "invalid response object"),
        ("event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"output\":[],\"status\":\"failed\"}}\n\n", "invalid terminal status"),
        ("data: {\"type\":\"response.completed\",\"response\":{\"output\":[],\"usage\":{\"output_tokens\":-1}}}\n\n", "invalid usage count"),
    ] {
        let actual = run_with_frame_prefix(
            fallback(response(tool("function_call", true), "completed"), false),
            config(), Some((1, "fixture transport connection closed")),
            "response.completed", prefix);
        let expected = if reason == "invalid terminal status" {
            "upstream response failed: the provider did not supply a failure cause".to_owned()
        } else { format!("streamed chunk has {reason}") };
        assert_eq!(actual.result.unwrap_err(), expected);
        assert!(actual.events.is_empty());
    assert_eq!((actual.requests.len(), actual.refunds), (3, 0));
    }
}

#[test]
fn malformed_fallback_completion_preserves_prior_published_chunk() {
    let mut replies = vec![Reply { streamed:false, at_budget:false,
        body:Some(response(text("first"), "incomplete")) }];
    replies.extend(fallback(response(tool("custom_tool_call", true), "completed"), false));
    let actual = run_with_frame_prefix(replies, config(),
        Some((2, "fixture transport connection closed")), "response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"output\":[null]}}\n\n");
    assert_eq!(actual.result.unwrap_err(), "streamed chunk has invalid output item");
    assert_eq!(actual.events, vec![(vec![text("first")], true)]);
    assert_eq!((actual.requests.len(), actual.refunds), (4, 0));
}
