use super::*;
use async_trait::async_trait;
use framer_core::ids::FramerUserId;
use framer_core::port::CastleToken;
use std::collections::VecDeque;

struct Reply {
    body: Value,
    at_budget: bool,
    error: Option<&'static str>,
}

fn tool(kind: &str, complete: bool) -> Value {
    let mut value = json!({"type":kind,"id":"item-fixture","call_id":"call-fixture",
        "name":"exec","namespace":"functions","status":"completed"});
    if kind == "custom_tool_call" {
        value["input"] = json!(if complete { "text(1);" } else { "const value = \"cut" });
    } else {
        value["arguments"] = json!(if complete { "{\"value\":1}" } else { "{\"value\":" });
    }
    value
}

fn reply(output: Value, status: Option<&str>, at_budget: bool) -> Reply {
    let mut body = json!({"id":"response-fixture","created_at":1,
        "output":[output],"usage":{"input_tokens":100,"output_tokens":200}});
    if let Some(status) = status { body["status"] = json!(status); }
    if status == Some("incomplete") {
        body["incomplete_details"] = json!({"reason":"max_output_tokens"});
    }
    Reply { body, at_budget, error: None }
}

fn transport_error() -> Reply {
    Reply { body: Value::Null, at_budget: false, error: Some("fixture transport connection closed") }
}

struct Fixture {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Value>>,
}

#[async_trait]
impl BrowserBridge for Fixture {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> { panic!("unexpected mint") }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> { panic!("unexpected login") }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> { panic!("unexpected login") }
    async fn seed_session(&self, _: &AccountId, cookie: &str) -> Result<(), String> {
        assert_eq!(cookie, "fixture-only");
        Ok(())
    }
    async fn ai_post(&self, _: &AccountId, path: &str, bearer: &str, body: &Value)
        -> Result<(u16, Option<String>, String), String>
    {
        assert_eq!(path, "/ai/v3/responses/");
        assert_eq!(bearer, "fixture-only");
        assert_eq!(body["request"]["stream"], false);
        assert_eq!(body["projectId"], "fixture-project");
        assert_eq!(body["sessionId"], "fixture-chat");
        self.requests.lock().unwrap().push(body.clone());
        let mut reply = self.replies.lock().unwrap().pop_front().expect("unexpected extra attempt");
        if let Some(error) = reply.error {
            if error == "fixture local cooldown" {
                return Ok((0, None, "velocity-paused: every device is cooling; retry in 599s".into()));
            }
            return Err(error.to_string());
        }
        if reply.at_budget {
            reply.body["usage"]["output_tokens"] = body["request"]["max_output_tokens"].clone();
        }
        Ok((200, None, reply.body.to_string()))
    }
}

struct Run {
    result: Result<AssembledResponses, String>,
    requests: Vec<Value>,
    emitted: Vec<Value>,
    elapsed: std::time::Duration,
}

fn config() -> ChainConfig {
    ChainConfig { chunk_tokens:1500, max_total_tokens:Some(4096), max_chunks:8,
        metered_fallback:false, ..ChainConfig::default() }
}

fn run(replies: Vec<Reply>, cfg: ChainConfig) -> Run {
    run_with_clock(replies, cfg, false)
}

fn run_with_clock(replies: Vec<Reply>, cfg: ChainConfig, paused: bool) -> Run {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all().start_paused(paused).build().unwrap();
    runtime.block_on(async {
        let started = tokio::time::Instant::now();
        let account = AccountId(uuid::Uuid::from_u128(1));
        let session = Session { active_user:FramerUserId("fixture-user".into()), cookie_header:"fixture-only".into() };
        let cache = Mutex::new(vec![(account, AiToken { bearer:"fixture-only".into(), expires_at_ms:i64::MAX })]);
        let browser = Fixture { replies:Mutex::new(replies.into()), requests:Mutex::new(Vec::new()) };
        let events = Mutex::new(Vec::new());
        let callback = |event| {
            if let ChainEvent::ResponsesItems { items, .. } = event {
                events.lock().unwrap().extend(items);
            }
        };
        let result = complete_long(&Transport::with_base("http://127.0.0.1:9".into()),
            &cache, &session, &browser, &account, &ProjectId("fixture-project".into()),
            json!({"model":"gpt-6-astra","service_tier":"priority","input":"fixture",
                "reasoning":{"effort":"max"},"tools":[{"type":"custom","name":"functions.exec"}]}),
            &cfg, "fixture-chat", Some(&callback)).await;
        assert!(browser.replies.lock().unwrap().is_empty(), "fixture expected more attempts");
        Run { result, requests:browser.requests.into_inner().unwrap(),
            emitted:events.into_inner().unwrap(), elapsed:started.elapsed() }
    })
}

#[test]
fn local_cooldown_after_output_waits_and_retries_only_unfinished_chunk() {
    let message = |text: &str, status: &str| json!({"type":"message", "id":"message-fixture",
        "role":"assistant", "status":status, "content":[{"type":"output_text", "text":text}]});
    let r = run_with_clock(vec![
        reply(message("first", "incomplete"), Some("incomplete"), true),
        Reply { body:Value::Null, at_budget:false, error:Some("fixture local cooldown") },
        reply(message("second", "completed"), Some("completed"), false),
    ], config(), true);
    let response = r.result.unwrap();
    assert_eq!(response.status, "completed");
    assert_eq!(response.output[0]["status"], "completed");
    assert_eq!(response.chunks, 2);
    assert_eq!(r.requests.len(), 3);
    assert_eq!(r.requests[1]["request"], r.requests[2]["request"]);
    assert_ne!(r.requests[0]["request"], r.requests[1]["request"]);
    assert_eq!(r.emitted.iter().map(|item| item["content"][0]["text"]
        .as_str().unwrap()).collect::<String>(), "firstsecond");
    assert!(r.elapsed >= std::time::Duration::from_secs(600));
    assert!(r.elapsed < std::time::Duration::from_secs(610));
}

fn recovered(kind: &str, status: Option<&str>) {
    let bad = tool(kind, false);
    let good = tool(kind, true);
    let r = run(vec![reply(bad.clone(), status, true), reply(good.clone(), Some("completed"), false)], config());
    let response = r.result.unwrap();
    assert_eq!(response.output, vec![good.clone()]);
    assert_eq!(r.emitted, vec![good]);
    assert_eq!(r.requests.len(), 2);
    assert_eq!(r.requests[0]["request"]["max_output_tokens"], 1500);
    assert_eq!(r.requests[1]["request"]["max_output_tokens"], 4000);
    assert!(!r.requests[1]["request"]["input"].as_array().unwrap().contains(&bad));
    assert_eq!(r.requests[0]["request"]["tools"], r.requests[1]["request"]["tools"]);
    assert_eq!(r.requests[0]["request"]["reasoning"], r.requests[1]["request"]["reasoning"]);
}

#[test]
fn completed_custom_call_at_budget_is_withheld_and_retried() {
    recovered("custom_tool_call", Some("completed"));
}

#[test]
fn absent_status_does_not_publish_a_truncated_custom_call() {
    recovered("custom_tool_call", None);
}

#[test]
fn completed_function_call_at_budget_is_withheld_and_retried() {
    recovered("function_call", Some("completed"));
}

#[test]
fn explicit_incomplete_tool_call_keeps_its_existing_repair_path() {
    recovered("custom_tool_call", Some("incomplete"));
}

#[test]
fn complete_call_below_budget_preserves_all_output_fields() {
    let good = tool("custom_tool_call", true);
    let r = run(vec![reply(good.clone(), Some("completed"), false)], config());
    let response = r.result.unwrap();
    assert_eq!(response.output, vec![good.clone()]);
    assert_eq!(response.output_tokens, 200);
    assert_eq!(response.status, "completed");
    assert_eq!(r.emitted, vec![good]);
    assert_eq!(r.requests.len(), 1);
}

#[test]
fn exhausted_retries_never_publish_partial_calls_even_with_metered_enabled() {
    for metered in [false, true] {
        let mut cfg = config();
        cfg.metered_fallback = metered;
        let replies = (0..3).map(|_| reply(tool("custom_tool_call", false), Some("completed"), true)).collect();
        let r = run(replies, cfg);
        assert!(r.result.unwrap_err().contains("two bounded retries"));
        assert_eq!(r.requests.len(), 3);
        assert!(r.emitted.is_empty());
    }
}

#[test]
fn final_chunk_cannot_turn_an_unfinished_tool_into_empty_success() {
    let mut cfg = config();
    cfg.max_chunks = 1;
    let r = run(vec![reply(tool("custom_tool_call", false), Some("completed"), true)], cfg);
    assert!(r.result.unwrap_err().contains("no remaining retry chunk"));
    assert!(r.emitted.is_empty());
    assert_eq!(r.requests.len(), 1);
}

fn text_item() -> Value {
    json!({"type":"message","id":"message-fixture","role":"assistant",
        "content":[{"type":"output_text","text":"Fixture prefix."}]})
}

#[test]
fn repair_preserves_prior_usage_and_the_remaining_client_budget() {
    let good = tool("custom_tool_call", true);
    let prefix = text_item();
    let r = run(vec![reply(prefix.clone(), Some("incomplete"), true),
        reply(tool("custom_tool_call", false), Some("completed"), true),
        reply(good.clone(), Some("completed"), false)], config());
    let response = r.result.unwrap();
    assert_eq!(response.output_tokens, 1700);
    assert_eq!(response.output, vec![prefix.clone(), good.clone()]);
    assert_eq!(r.emitted, vec![prefix, good]);
    assert_eq!(r.requests.len(), 3);
    assert_eq!(r.requests[1]["request"]["max_output_tokens"], 2596);
    assert_eq!(r.requests[2]["request"]["max_output_tokens"], 2596);
}

#[test]
fn repair_does_not_expand_an_explicit_small_client_budget() {
    let mut cfg = config();
    cfg.max_total_tokens = Some(256);
    let r = run(vec![reply(tool("custom_tool_call", false), Some("completed"), true),
        reply(tool("custom_tool_call", true), Some("completed"), false)], cfg);
    assert!(r.result.is_ok());
    assert_eq!(r.requests.len(), 2);
    for request in &r.requests { assert_eq!(request["request"]["max_output_tokens"], 256); }
}

#[test]
fn completed_text_at_budget_is_not_reinterpreted_as_a_tool_fragment() {
    let text = text_item();
    let r = run(vec![reply(text.clone(), Some("completed"), true)], config());
    assert_eq!(r.result.unwrap().output, vec![text.clone()]);
    assert_eq!(r.emitted, vec![text]);
    assert_eq!(r.requests.len(), 1);
}

#[test]
fn transport_retry_boundary_tool_uses_actual_successful_budget() {
    let bad = tool("custom_tool_call", false);
    let good = tool("custom_tool_call", true);
    let r = run(vec![transport_error(), reply(bad.clone(), Some("completed"), true),
        reply(good.clone(), Some("completed"), false)], config());
    let response = r.result.unwrap();
    assert_eq!(response.output, vec![good.clone()]);
    assert_eq!(response.output_tokens, 200);
    assert_eq!(r.emitted, vec![good]);
    assert_eq!(r.requests.len(), 3);
    assert_eq!(r.requests[0]["request"]["max_output_tokens"], 1500);
    assert_eq!(r.requests[1]["request"]["max_output_tokens"], 750);
    assert_eq!(r.requests[2]["request"]["max_output_tokens"], 4000);
    assert!(!r.requests[2]["request"]["input"].as_array().unwrap().contains(&bad));
}

#[test]
fn transport_retry_respects_small_remaining_client_budget() {
    let mut cfg = config();
    cfg.max_total_tokens = Some(256);
    let good = tool("custom_tool_call", true);
    let r = run(vec![transport_error(),
        reply(tool("custom_tool_call", false), Some("completed"), true),
        reply(good.clone(), Some("completed"), false)], cfg);
    let response = r.result.unwrap();
    assert_eq!(response.output, vec![good.clone()]);
    assert_eq!(response.output_tokens, 200);
    assert_eq!(r.emitted, vec![good]);
    assert_eq!(r.requests.len(), 3);
    for request in &r.requests { assert_eq!(request["request"]["max_output_tokens"], 256); }
}

#[test]
fn transport_retry_after_prefix_preserves_remaining_budget_and_usage() {
    let mut cfg = config();
    cfg.max_total_tokens = Some(1756);
    let good = tool("custom_tool_call", true);
    let prefix = text_item();
    let r = run(vec![reply(prefix.clone(), Some("incomplete"), true), transport_error(),
        reply(tool("custom_tool_call", false), Some("completed"), true),
        reply(good.clone(), Some("completed"), false)], cfg);
    let response = r.result.unwrap();
    assert_eq!(response.output_tokens, 1700);
    assert_eq!(response.output, vec![prefix.clone(), good.clone()]);
    assert_eq!(r.emitted, vec![prefix, good]);
    assert_eq!(r.requests.len(), 4);
    assert_eq!(r.requests[0]["request"]["max_output_tokens"], 1500);
    for request in &r.requests[1..] { assert_eq!(request["request"]["max_output_tokens"], 256); }
}

#[test]
fn successful_transport_retry_below_its_budget_delivers_without_repair() {
    let good = tool("function_call", true);
    let r = run(vec![transport_error(), reply(good.clone(), Some("completed"), false)], config());
    let response = r.result.unwrap();
    assert_eq!(response.output, vec![good.clone()]);
    assert_eq!(response.output_tokens, 200);
    assert_eq!(r.emitted, vec![good]);
    assert_eq!(r.requests.len(), 2);
    assert_eq!(r.requests[0]["request"]["max_output_tokens"], 1500);
    assert_eq!(r.requests[1]["request"]["max_output_tokens"], 750);
}
