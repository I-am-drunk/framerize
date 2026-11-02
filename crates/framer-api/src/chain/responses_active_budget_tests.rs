use super::*;
use async_trait::async_trait;
use framer_core::{ids::FramerUserId, port::CastleToken};
use std::{collections::VecDeque, sync::{Arc, atomic::{AtomicUsize, Ordering}}};

struct Reply {
    seconds: u64,
    result: Result<(u16, String), &'static str>,
}

fn failure(seconds: u64, error: &'static str) -> Reply {
    Reply { seconds, result: Err(error) }
}

fn answer(seconds: u64, partial: bool) -> Reply {
    Reply { seconds, result: Ok((200, json!({
        "id":"fixture-response", "status":if partial { "incomplete" } else { "completed" },
        "incomplete_details":if partial { json!({"reason":"max_output_tokens"}) } else { Value::Null },
        "output":[{"type":"message","role":"assistant",
            "content":[{"type":"output_text","text":"answer"}]}],
        "usage":{"input_tokens":1,"output_tokens":10}
    }).to_string())) }
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
    async fn seed_session(&self, account: &AccountId, cookie: &str) -> Result<(), String> {
        assert_eq!(*account, AccountId(uuid::Uuid::from_u128(1)));
        assert_eq!(cookie, "fixture-only");
        Ok(())
    }
    async fn ai_post(&self, account: &AccountId, path: &str, bearer: &str, body: &Value)
        -> Result<(u16, Option<String>, String), String>
    {
        assert_eq!(*account, AccountId(uuid::Uuid::from_u128(1)));
        assert_eq!(path, "/ai/v3/responses/");
        assert_eq!(bearer, "fixture-only");
        assert_eq!(body["projectId"], "fixture-project");
        assert_eq!(body["sessionId"], "fixture-chat");
        self.requests.lock().unwrap().push(body["request"].clone());
        let reply = self.replies.lock().unwrap().pop_front().expect("unexpected attempt");
        // Keep boundary observations independent of timer rounding.
        tokio::time::advance(std::time::Duration::from_secs(reply.seconds)).await;
        reply.result.map(|(status, text)| (status, None, text)).map_err(str::to_string)
    }
}

struct Outcome {
    result: Result<AssembledResponses, String>,
    requests: Vec<Value>,
    events: Vec<Vec<Value>>,
    rate_retries: usize,
}

async fn run(replies: Vec<Reply>) -> Outcome {
    let account = AccountId(uuid::Uuid::from_u128(1));
    let session = Session { active_user:FramerUserId("fixture-user".into()), cookie_header:"fixture-only".into() };
    let cache = Mutex::new(vec![(account, AiToken::for_session(&session, "fixture-only".into(), i64::MAX))]);
    let browser = Fixture { replies:Mutex::new(replies.into()), requests:Mutex::new(Vec::new()) };
    let retries = Arc::new(AtomicUsize::new(0));
    let counter = retries.clone();
    let cfg = ChainConfig { chunk_tokens:1500, max_total_tokens:Some(4096), max_chunks:4,
        metered_fallback:true, on_429:Some(Arc::new(move || { counter.fetch_add(1, Ordering::SeqCst); })),
        ..ChainConfig::default() };
    let events = Mutex::new(Vec::new());
    let callback = |event| {
        if let ChainEvent::ResponsesItems { items, .. } = event { events.lock().unwrap().push(items); }
    };
    let result = complete_long(&Transport::with_base("http://127.0.0.1:9".into()),
        &cache, &session, &browser, &account, &ProjectId("fixture-project".into()),
        json!({"model":"gpt-6-astra", "input":"fixture", "service_tier":"priority"}),
        &cfg, "fixture-chat", Some(&callback)).await;
    assert!(browser.replies.lock().unwrap().is_empty(), "unconsumed replies");
    Outcome { result, requests:browser.requests.into_inner().unwrap(),
        events:events.into_inner().unwrap(), rate_retries:retries.load(Ordering::SeqCst) }
}

fn assert_expired(outcome: Outcome, requests: usize) {
    let error = outcome.result.unwrap_err();
    assert!(error.starts_with("chain exceeded 300s active-time budget after 0 chunks"), "{error}");
    assert!(error.contains("last failure:"), "{error}");
    assert_eq!(outcome.requests.len(), requests);
    assert!(outcome.events.is_empty());
}

#[tokio::test(start_paused = true)]
async fn rate_retry_does_not_dispatch_after_active_budget() {
    let replies = (0..4).map(|_| Reply { seconds:80,
        result:Ok((429, "too many concurrent AI requests for this workspace".into())) }).collect();
    let outcome = run(replies).await;
    assert_eq!(outcome.rate_retries, 4);
    assert_expired(outcome, 4);
}

#[tokio::test(start_paused = true)]
async fn transport_retry_does_not_dispatch_after_active_budget() {
    assert_expired(run(vec![failure(301, "fixture connection closed")]).await, 1);
}

#[tokio::test(start_paused = true)]
async fn metered_fallback_does_not_dispatch_after_active_budget() {
    let outcome = run(vec![failure(150, "fixture connection closed"),
        failure(151, "fixture connection closed")]).await;
    assert!(outcome.requests.iter().all(|request| request["stream"] == false));
    assert_expired(outcome, 2);
}

#[tokio::test(start_paused = true)]
async fn exact_budget_does_not_start_another_attempt() {
    assert_expired(run(vec![failure(300, "fixture connection closed")]).await, 1);
}

#[tokio::test(start_paused = true)]
async fn request_admitted_before_budget_keeps_its_late_success() {
    let outcome = run(vec![failure(299, "fixture connection closed"), answer(2, false)]).await;
    assert_eq!(outcome.requests.len(), 2);
    assert_eq!(outcome.events.len(), 1);
    assert_eq!(outcome.result.unwrap().status, "completed");
}

#[tokio::test(start_paused = true)]
async fn successful_inflight_response_survives_elapsed_budget() {
    let outcome = run(vec![answer(301, false)]).await;
    assert_eq!(outcome.requests.len(), 1);
    assert_eq!(outcome.events.len(), 1);
    assert_eq!(outcome.result.unwrap().status, "completed");
}

#[tokio::test(start_paused = true)]
async fn expired_budget_preserves_nonretryable_error() {
    let outcome = run(vec![failure(301, "credits-exhausted")]).await;
    assert_eq!(outcome.requests.len(), 1);
    assert_eq!(outcome.result.unwrap_err(), "credits-exhausted");
    assert!(outcome.events.is_empty());
}

#[tokio::test(start_paused = true)]
async fn local_cooldown_wait_does_not_consume_active_budget() {
    let outcome = run(vec![answer(0, true), failure(0, "account-cooling:600"), answer(0, false)]).await;
    assert_eq!(outcome.requests.len(), 3);
    assert_eq!(outcome.events.len(), 2);
    assert_eq!(outcome.result.unwrap().status, "completed");
}

#[tokio::test(start_paused = true)]
async fn a_late_local_refusal_cannot_start_a_hidden_retry() {
    let result = run(vec![answer(0, true), failure(301, "account-cooling:10")]).await;
    let error = result.result.unwrap_err();
    assert!(error.contains("300s active-time budget"), "{error}");
    assert!(error.contains("account-cooling:10"), "{error}");
    assert_eq!(result.requests.len(), 2);
    assert_eq!(result.events.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn rate_retry_with_active_budget_preserves_callback_and_success() {
    let outcome = run(vec![Reply { seconds:1, result:Ok((429,
        "too many concurrent AI requests for this workspace".into())) },
        answer(1, false)]).await;
    assert_eq!(outcome.requests.len(), 2);
    assert_eq!(outcome.rate_retries, 1);
    assert_eq!(outcome.events.len(), 1);
    assert_eq!(outcome.result.unwrap().status, "completed");
}

#[tokio::test(start_paused = true)]
async fn slow_successful_responses_are_delivered() {
    let outcome = run(vec![answer(20, true), answer(20, false)]).await;
    assert_eq!(outcome.requests.len(), 2);
    assert_eq!(outcome.events.len(), 2);
    assert_eq!(outcome.result.unwrap().status, "completed");
}

#[tokio::test(start_paused = true)]
async fn provider_rate_limit_does_not_mark_workspace_saturated() {
    let outcome = run(vec![Reply { seconds:1, result:Ok((429,
        "upstream_provider_shared_pool".into())) }, answer(1, false)]).await;
    assert_eq!(outcome.requests.len(), 2);
    assert_eq!(outcome.rate_retries, 0);
    assert_eq!(outcome.result.unwrap().status, "completed");
}
