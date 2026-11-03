use super::*;
use async_trait::async_trait;
use framer_core::ids::FramerUserId;
use framer_core::port::CastleToken;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "messages_checkpoint_tests.rs"]
mod checkpoint_tests;

#[path = "messages_opus48_checkpoint_tests.rs"]
mod opus48_checkpoint_tests;

#[path = "messages_opus55_checkpoint_tests.rs"]
mod opus55_checkpoint_tests;

#[path = "messages_json_chain_tests.rs"]
mod json_tests;

#[path = "messages_config_tests.rs"]
mod config_tests;

#[path = "messages_content_chain_tests.rs"]
mod content_tests;

#[path = "messages_usage_tests.rs"]
mod usage_tests;

#[path = "messages_validation_chain_tests.rs"]
mod validation_tests;

#[path = "messages_retry_policy_tests.rs"]
mod retry_policy_tests;

#[path = "messages_stream_usage_chain_tests.rs"]
mod stream_usage_tests;

#[path = "messages_output_usage_tests.rs"]
mod output_usage_tests;

#[path = "messages_signature_chain_tests.rs"]
mod signature_tests;

#[path = "messages_metadata_chain_tests.rs"]
mod metadata_tests;

#[path = "messages_sse_event_chain_tests.rs"]
mod sse_events;

#[path = "messages_line_endings_chain_tests.rs"]
mod line_endings;

#[path = "messages_thinking_chain_tests.rs"]
mod thinking_tests;

#[path = "messages_thinking_budget_tests.rs"]
mod thinking_budget_tests;

#[path = "messages_default_thinking_tests.rs"]
mod default_thinking_tests;

#[path = "messages_adaptive_streaming_tests.rs"]
mod adaptive_streaming_tests;

#[path = "messages_timing_tests.rs"]
mod timing_tests;

struct Observed {
    result: Result<AssembledResponse, String>,
    emitted: Vec<Value>,
    requests: Vec<Value>,
    refunds: usize,
    frames: Vec<String>,
}

fn run(replies: Vec<Reply>, max_chunks: u32) -> Observed {
    run_with_config(replies, ChainConfig { chunk_tokens: 600, max_chunks,
        max_total_tokens: Some(4096), metered_fallback: true,
        ..ChainConfig::default() })
}

fn run_with_config(replies: Vec<Reply>, config: ChainConfig) -> Observed {
    run_with_delays(replies, config, Vec::new())
}

fn run_with_delays(replies: Vec<Reply>, config: ChainConfig,
    delays: Vec<std::time::Duration>) -> Observed {
    run_with_payload(replies, config, delays, json!({"model":"claude-sonnet-4",
        "messages":[{"role":"user","content":"fixture"}]}))
}

fn run_with_payload(replies: Vec<Reply>, config: ChainConfig,
    delays: Vec<std::time::Duration>, payload: Value) -> Observed {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all().start_paused(true).build().unwrap();
    runtime.block_on(async {
        let account = AccountId(uuid::Uuid::from_u128(1));
        let session = Session { active_user: FramerUserId("fixture-user".into()),
            cookie_header: "fixture-only".into() };
        let cache = Mutex::new(vec![(account,
            AiToken::for_session(&session, "fixture-only".into(), i64::MAX))]);
        let browser = Fixture { replies: Mutex::new(replies.into()),
            requests: Mutex::new(Vec::new()), refunds: AtomicUsize::new(0),
            delays: Mutex::new(delays.into()) };
        let emitted = Mutex::new(Vec::new());
        use framer_core::convert::{self, ClientDialect, SseComposer};
        let dialect = ClientDialect::AnthropicMessages;
        let composer = Mutex::new(SseComposer::new(dialect, "client-fixture", "claude-sonnet-4"));
        let frames = Mutex::new(convert::stream_open(dialect, "claude-sonnet-4", "client-fixture"));
        let callback = |event| {
            if let ChainEvent::Chunk(blocks) = event {
                frames.lock().unwrap().extend(composer.lock().unwrap().push_chunk(&blocks));
                emitted.lock().unwrap().extend(blocks);
            }
        };
        let transport = Transport::with_base("http://127.0.0.1:9".into());
        let result = complete_long(&transport, &cache, &session, &browser, &account,
            &ProjectId("fixture-project".into()), payload,
            &config, "fixture-chat", Some(&callback)).await;
        assert!(browser.replies.lock().unwrap().is_empty(), "missing expected request");
        assert!(browser.delays.lock().unwrap().is_empty(), "unused request delay");
        if let Ok(response) = &result {
            frames.lock().unwrap().extend(composer.lock().unwrap()
                .finish(response, "claude-sonnet-4", false));
        }
        Observed { result, emitted: emitted.into_inner().unwrap(),
            frames: frames.into_inner().unwrap(),
            requests: browser.requests.into_inner().unwrap(), refunds: browser.refunds.load(Ordering::SeqCst) }
    })
}

struct Reply {
    streaming: bool,
    status: u16,
    result: Result<String, String>,
}

fn failed_request() -> Reply {
    Reply { streaming: false, status: 200, result: Err("fixture transport closed".into()) }
}

fn tool() -> Value {
    json!({"type":"tool_use","id":"tool-fixture","name":"fixture","input":{"value":42}})
}

fn stream(block: Value, reason: &str) -> String {
    [
        json!({"type":"message_start","message":{"id":"stream-fixture",
            "usage":{"input_tokens":17}}}),
        json!({"type":"content_block_start","index":0,"content_block":block}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":reason,"stop_sequence":null},
            "usage":{"output_tokens":80}}),
        json!({"type":"message_stop"}),
    ].iter().map(|event| format!("data: {event}\n\n")).collect()
}

fn streamed(text: String) -> Reply {
    Reply { streaming: true, status: 200, result: Ok(text) }
}

fn buffered(block: Value, reason: &str) -> Reply {
    Reply { streaming: false, status: 200, result: Ok(json!({"id":"buffered-fixture",
        "content":[block],"stop_reason":reason,
        "usage":{"input_tokens":19,"output_tokens":80}}).to_string()) }
}

#[test]
fn final_retry_chunk_cannot_turn_unfinished_tool_into_empty_success() {
    for max_chunks in [1, 2] {
        let replies = (0..max_chunks).map(|_| buffered(tool(), "max_tokens")).collect();
        let observed = run(replies, max_chunks);
        assert!(observed.result.unwrap_err().contains("no remaining retry chunk"));
        assert!(observed.emitted.is_empty());
        assert_eq!(observed.requests.len(), max_chunks as usize);
        assert_eq!(observed.refunds, 0);
    }
}

#[test]
fn transport_fallback_never_publishes_a_truncated_tool_stream() {
    let good = stream(tool(), "tool_use");
    let bad = good.replace("data: {\"type\":\"message_stop\"}\n\n", "");
    assert_ne!(bad, good);
    let observed = run(vec![failed_request(), failed_request(), streamed(bad)], 1);
    assert!(observed.result.unwrap_err().contains("without message_stop"));
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.requests.len(), 3);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn transport_fallback_never_publishes_invalid_tool_json() {
    let valid = stream(tool(), "tool_use");
    let mut values: Vec<Value> = valid.lines().filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).unwrap()).collect();
    values.insert(2, json!({"type":"content_block_delta","index":0,
        "delta":{"type":"input_json_delta","partial_json":"{\"value\":"}}));
    let bad = values.iter().map(|event| format!("data: {event}\n\n")).collect();
    let observed = run(vec![failed_request(), failed_request(), streamed(bad)], 1);
    assert!(observed.result.unwrap_err().contains("invalid tool input JSON"));
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.requests.len(), 3);
}

#[test]
fn transport_fallback_preserves_a_complete_tool_response() {
    let observed = run(vec![failed_request(), failed_request(),
        streamed(stream(tool(), "tool_use"))], 1);
    let response = observed.result.unwrap();
    assert_eq!(response.id, "stream-fixture");
    assert_eq!(response.content, vec![tool()]);
    assert_eq!(response.stop_reason, "tool_use");
    assert_eq!((response.input_tokens, response.output_tokens), (17, 80));
    assert_eq!((response.chunks, response.metered_chunks), (1, 1));
    assert_eq!(observed.emitted, vec![tool()]);
    assert_eq!(observed.requests.len(), 3);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn repeated_tool_cut_fallback_rejects_upstream_error_before_emission() {
    let bad = stream(tool(), "tool_use")
        + "data: {\"type\":\"error\",\"error\":{\"message\":\"fixture overload\"}}\n\n";
    let observed = run(vec![buffered(tool(), "max_tokens"),
        buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens"), streamed(bad)], 3);
    assert!(observed.result.unwrap_err().contains("upstream error"));
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.requests.len(), 4);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn completed_text_limit_fallback_continues_with_its_content_and_usage() {
    let first = json!({"type":"text","text":"first"});
    let second = json!({"type":"text","text":"second"});
    let observed = run(vec![failed_request(), failed_request(),
        streamed(stream(first.clone(), "max_tokens")), buffered(second.clone(), "end_turn")], 2);
    let response = observed.result.unwrap();
    assert_eq!(response.content, vec![json!({"type":"text","text":"firstsecond"})]);
    assert_eq!(response.stop_reason, "end_turn");
    assert_eq!((response.input_tokens, response.output_tokens), (19, 160));
    assert_eq!((response.chunks, response.metered_chunks), (2, 1));
    assert_eq!(observed.emitted, vec![first.clone(), second]);
    assert_eq!(observed.requests.len(), 4);
    assert_eq!(observed.requests[3]["request"]["messages"][1]["content"], json!([first]));
    assert_eq!(observed.refunds, 0);
}

#[test]
fn final_text_fallback_preserves_its_token_limit_status() {
    let block = json!({"type":"text","text":"partial"});
    let observed = run(vec![failed_request(), failed_request(),
        streamed(stream(block.clone(), "max_tokens"))], 1);
    let response = observed.result.unwrap();
    assert_eq!(response.content, vec![block.clone()]);
    assert_eq!(response.stop_reason, "max_tokens");
    assert_eq!((response.chunks, response.metered_chunks), (1, 1));
    assert_eq!(observed.emitted, vec![block]);
    assert_eq!(observed.requests.len(), 3);
}

#[test]
fn final_tool_repair_fallback_preserves_text_limit_status() {
    let block = json!({"type":"text","text":"partial"});
    let observed = run(vec![buffered(tool(), "max_tokens"),
        buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens"),
        streamed(stream(block.clone(), "max_tokens"))], 3);
    let response = observed.result.unwrap();
    assert_eq!(response.stop_reason, "max_tokens");
    assert_eq!((response.chunks, response.metered_chunks), (3, 1));
    assert_eq!(observed.emitted, vec![block]);
    assert_eq!(observed.requests.len(), 4);
}

fn capped_config(max_chunks: u32) -> ChainConfig {
    ChainConfig { chunk_tokens: 80, max_chunks, max_total_tokens: Some(256),
        metered_fallback: true, ..ChainConfig::default() }
}

#[test]
fn rate_retry_preserves_the_remaining_client_budget() {
    let prefix = json!({"type":"text","text":"prefix"});
    let limited = Reply { streaming: false, status: 429, result: Ok("fixture full".into()) };
    let observed = run_with_config(vec![buffered(prefix, "max_tokens"), limited,
        buffered(tool(), "tool_use")], capped_config(2));
    assert_eq!(observed.result.unwrap().output_tokens, 160);
    assert_eq!(observed.requests[0]["request"]["max_tokens"], 80);
    assert_eq!(observed.requests.len(), 3);
    for request in &observed.requests[1..] {
        assert_eq!(request["request"]["max_tokens"], 176);
    }
}

#[test]
fn transport_retry_preserves_the_remaining_client_budget() {
    let prefix = json!({"type":"text","text":"prefix"});
    let observed = run_with_config(vec![buffered(prefix, "max_tokens"),
        failed_request(), buffered(tool(), "tool_use")], capped_config(2));
    assert_eq!(observed.result.unwrap().output_tokens, 160);
    assert_eq!(observed.requests.len(), 3);
    for request in &observed.requests[1..] {
        assert_eq!(request["request"]["max_tokens"], 176);
    }
}

#[test]
fn metered_transport_fallback_preserves_the_remaining_client_budget() {
    let prefix = json!({"type":"text","text":"prefix"});
    let observed = run_with_config(vec![buffered(prefix, "max_tokens"), failed_request(),
        failed_request(), streamed(stream(tool(), "tool_use"))], capped_config(2));
    assert_eq!(observed.result.unwrap().output_tokens, 160);
    assert_eq!(observed.requests.len(), 4);
    for request in &observed.requests[1..] {
        assert_eq!(request["request"]["max_tokens"], 176);
    }
}

#[test]
fn repeated_tool_cut_fallback_does_not_expand_a_small_client_cap() {
    let observed = run_with_config(vec![buffered(tool(), "max_tokens"),
        buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens"),
        streamed(stream(tool(), "tool_use"))], capped_config(3));
    assert_eq!(observed.result.unwrap().content, vec![tool()]);
    assert_eq!(observed.requests.len(), 4);
    assert_eq!(observed.requests[0]["request"]["max_tokens"], 80);
    for request in &observed.requests[1..] {
        assert_eq!(request["request"]["max_tokens"], 256);
    }
}

struct Fixture {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Value>>,
    refunds: AtomicUsize,
    delays: Mutex<VecDeque<std::time::Duration>>,
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
    async fn seed_session(&self, _: &AccountId, cookie: &str) -> Result<(), String> {
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
            self.refunds.fetch_add(1, Ordering::SeqCst);
            return Ok((200, None, "{\"granted\":false}".into()));
        }
        assert_eq!(path, "/ai/v3/messages/");
        self.requests.lock().unwrap().push(body.clone());
        let reply = self.replies.lock().unwrap().pop_front().expect("unexpected attempt");
        assert_eq!(body["request"]["stream"], reply.streaming);
        let delay = self.delays.lock().unwrap().pop_front();
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        reply.result.map(|text| (reply.status, None, text))
    }
}
