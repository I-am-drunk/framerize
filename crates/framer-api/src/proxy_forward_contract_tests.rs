//! Recorded legacy Forward contract. No live accounts or browser processes.
//! Launch category-specific groups in separate processes; never mutate env here.
use super::*;
use async_trait::async_trait;
use framer_core::{FramerUserId, port::{CastleToken, ProxyResponse}};
use futures::StreamExt;
use framer_protocol::forward::AuthorizedForward;
use std::{collections::VecDeque, sync::atomic::{AtomicBool, Ordering}};

const PATH: &str = "/ai/v3/responses/";
const REJECTED: &str = "model not allowed for this feature";
const OLD: &str = "fixture-original-bearer";
const FRESH: &str = "fixture-newer-session-bearer";
const BODY: &str = "data: fixture opaque body\n\n";
type Reply = Result<(u16, Option<String>, String), String>;

#[derive(Clone, Debug)]
struct Post {
    account: AccountId, path: String, bearer: String, body: Value,
    callback: Option<bool>,
}

#[derive(Clone, Debug)]
enum Event { Seed(AccountId, String), Post(Post), Queue(AccountId) }

struct Browser {
    replies: Mutex<VecDeque<Reply>>, events: Mutex<Vec<Event>>,
    stopped: AtomicBool, cancel_after_post: Option<usize>,
}

#[async_trait]
impl BrowserBridge for Browser {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        panic!("unexpected standalone Castle mint")
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("unexpected login")
    }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> {
        panic!("unexpected login")
    }
    async fn seed_session(&self, account: &AccountId, cookie: &str) -> Result<(), String> {
        self.events.lock().unwrap().push(Event::Seed(*account, cookie.into()));
        Ok(())
    }
    async fn ai_post_cancellable(&self, account: &AccountId, path: &str,
        bearer: &str, body: &Value, cancelled: Option<&(dyn Fn() -> bool + Send + Sync)>) -> Reply {
        let callback = cancelled.map(|check| check());
        assert_ne!(callback, Some(true), "dispatch after cancellation");
        let mut events = self.events.lock().unwrap();
        events.push(Event::Post(Post { account: *account, path: path.into(),
            bearer: bearer.into(), body: body.clone(), callback }));
        let count = events.iter().filter(|event| matches!(event, Event::Post(_))).count();
        if self.cancel_after_post == Some(count) { self.stopped.store(true, Ordering::SeqCst); }
        self.replies.lock().unwrap().pop_front().expect("unexpected extra post")
    }
    async fn last_queue_wait_ms(&self, account: &AccountId) -> u64 {
        self.events.lock().unwrap().push(Event::Queue(*account));
        // Queue lookup follows the completed inference. This zero matches today's host,
        // but does not prove correct attribution of nonzero per-post timing.
        0 // Timing redesign is outside S2a.
    }
}

struct Fixture { transport: Transport, session: Session, account: AccountId, browser: Browser }

impl Fixture {
    fn new(replies: Vec<Reply>) -> Self {
        // A broken retry must never discover a configured external JWT relay.
        assert!(std::env::var("FRAMERIZE_JWT_RELAY_URL").unwrap_or_default().is_empty());
        let account = AccountId(uuid::Uuid::new_v4());
        let session = Session { active_user: FramerUserId(format!("fixture-{}", account.0)),
            cookie_header: format!("fixture-session={}", account.0) };
        let transport = Transport::control_for_loopback("127.0.0.1:9".parse().unwrap(),
            crate::session_auth::Egress::Direct).unwrap();
        transport.authorization.accounts.lock().unwrap().push((account,
            AiToken::for_session(&session, OLD.into(), i64::MAX)));
        Self { transport, session, account, browser: Browser {
            replies: Mutex::new(replies.into()), events: Mutex::new(Vec::new()),
            stopped: AtomicBool::new(false), cancel_after_post: None } }
    }

    fn newer_session_token(&self) {
        // Exercise the real 401 branch without a network mint. auth::retry_tests
        // separately proves invalidation/remint, including failed/repeated 401s.
        self.transport.authorization.sessions.lock().unwrap().insert(
            (self.session.cookie_header.clone(), "/auth/ai/access-token".into()),
            AiToken::for_session(&self.session, FRESH.into(), i64::MAX));
    }

    async fn run(&self, payload: Value, supplied: bool) -> Result<ProxyResponse, String> {
        let check = || self.browser.stopped.load(Ordering::SeqCst);
        let cancelled: Option<&(dyn Fn() -> bool + Send + Sync)> =
            if supplied { Some(&check) } else { None };
        let project = ProjectId("fixture-project".into());
        let forward = crate::forward::ResponsesForward::new(
            &self.transport, &self.transport.authorization.accounts,
            &self.session, &self.browser, &self.account, &project,
            "fixture-chat", cancelled, None);
        forward.forward("translate", payload, 7).await.map(|reply| ProxyResponse {
            status: reply.status, content_type: reply.content_type,
            queue_wait_ms: reply.queue_wait_ms,
            stream: Box::pin(futures::stream::once(async move { Ok(reply.body) })),
        })
    }

    fn posts(&self) -> Vec<Post> {
        self.browser.events.lock().unwrap().iter().filter_map(|event| match event {
            Event::Post(post) => Some(post.clone()), _ => None,
        }).collect()
    }

    fn trace(&self, expected: &[&str]) {
        let events = self.browser.events.lock().unwrap();
        let actual: Vec<_> = events.iter().map(|event| match event {
            Event::Seed(account, cookie) => {
                assert_eq!(account, &self.account);
                assert_eq!(cookie, &self.session.cookie_header);
                "seed"
            }
            Event::Post(post) => { assert_eq!(post.account, self.account); "post" }
            Event::Queue(account) => { assert_eq!(account, &self.account); "queue" }
        }).collect();
        assert_eq!(actual, expected);
        assert!(self.browser.replies.lock().unwrap().is_empty(), "unconsumed script");
    }
}

fn ok(status: u16, body: &str) -> Reply { Ok((status, None, body.into())) }
fn category() -> String {
    std::env::var("FRAMERIZE_FEATURE_CATEGORY").unwrap_or_else(|_| "translation".into())
}
fn payload(streaming: bool) -> Value {
    json!({"model":"fixture-model", "stream":streaming, "input":[
        {"role":"user", "content":"earlier"}, {"role":"assistant", "content":"prior"},
        {"role":"user", "content":[{"type":"input_text", "text":"last"},
            {"input_text":"question"}]}], "tools":[{"type":"custom", "name":"fixture-tool"}],
        "unknown_option":{"preserve":true}})
}

fn envelope(post: &Post, payload: &Value, category: &str, bearer: &str, supplied: bool) {
    assert_eq!(post.path, PATH);
    assert_eq!(post.bearer, bearer);
    assert_eq!(post.callback, supplied.then_some(false));
    assert!(!post.body["turnId"].as_str().unwrap().is_empty());
    assert_eq!(post.body, json!({"projectId":"fixture-project",
        "featureCategory":category,
        "featureName":if category == "translation" { "translation" } else { "agents" },
        "model":payload["model"], "step":"translate", "sessionId":"fixture-chat",
        "stepIndex":7, "turnId":post.body["turnId"], "userPrompt":"last question",
        "request":payload}));
}

async fn success(mut response: ProxyResponse, status: u16, streaming: bool) {
    assert_eq!(response.status, status);
    assert_eq!(response.content_type, if streaming { "text/event-stream" } else { "application/json" });
    assert_eq!(response.queue_wait_ms, 0);
    assert_eq!(response.stream.next().await.unwrap().unwrap().as_ref(), BODY.as_bytes());
    assert!(response.stream.next().await.is_none());
}

#[tokio::test]
async fn common_streamed_envelope_never_makes_an_automatic_refund() {
    for supplied in [false, true] {
        let fixture = Fixture::new(vec![ok(201, BODY)]);
        let payload = payload(true);
        success(fixture.run(payload.clone(), supplied).await.unwrap(), 201, true).await;
        fixture.trace(&["seed", "post", "queue"]);
        let posts = fixture.posts();
        envelope(&posts[0], &payload, &category(), OLD, supplied);
        assert_eq!(posts.len(), 1);
    }
}

#[tokio::test]
async fn common_nonstream_values_preserve_payload_without_refund() {
    for stream in [None, Some(json!(false)), Some(Value::Null), Some(json!("true"))] {
        let fixture = Fixture::new(vec![ok(202, BODY)]);
        let mut payload = payload(false);
        if let Some(value) = stream { payload["stream"] = value; }
        else { payload.as_object_mut().unwrap().remove("stream"); }
        success(fixture.run(payload.clone(), true).await.unwrap(), 202, false).await;
        fixture.trace(&["seed", "post", "queue"]);
        envelope(&fixture.posts()[0], &payload, &category(), OLD, true);
    }
}

#[tokio::test]
async fn common_cancellation_before_forward_has_no_browser_effect() {
    let fixture = Fixture::new(vec![]);
    fixture.browser.stopped.store(true, Ordering::SeqCst);
    assert_eq!(fixture.run(payload(true), true).await.err().unwrap(), "client cancelled");
    fixture.trace(&[]);
}

#[tokio::test]
async fn common_cancellation_after_completion_skips_refund_but_keeps_body() {
    let mut fixture = Fixture::new(vec![ok(200, BODY)]);
    fixture.browser.cancel_after_post = Some(1);
    success(fixture.run(payload(true), true).await.unwrap(), 200, true).await;
    fixture.trace(&["seed", "post", "queue"]);
}

#[tokio::test]
async fn translation_cancellation_after_rejection_prevents_retry() {
    assert_eq!(category(), "translation");
    for status in [400, 401] {
        let mut fixture = Fixture::new(vec![ok(status, REJECTED)]);
        fixture.browser.cancel_after_post = Some(1);
        assert_eq!(fixture.run(payload(true), true).await.err().unwrap(), "client cancelled");
        fixture.trace(&["seed", "post"]);
    }
}

#[tokio::test]
async fn common_transport_failure_preserves_cleanup_signal_without_more_work() {
    let error = "host-lifecycle-unconfirmed: fixture request completion unknown";
    for supplied in [false, true] {
        let fixture = Fixture::new(vec![Err(error.into())]);
        assert_eq!(fixture.run(payload(true), supplied).await.err().unwrap(), error);
        fixture.trace(&["seed", "post"]);
    }
}

#[tokio::test]
async fn translation_category_fallback_records_new_turn_without_refund() {
    assert_eq!(category(), "translation");
    let fixture = Fixture::new(vec![ok(400, REJECTED), ok(200, BODY)]);
    success(fixture.run(payload(true), true).await.unwrap(), 200, true).await;
    fixture.trace(&["seed", "post", "seed", "post", "queue"]);
    let posts = fixture.posts();
    envelope(&posts[0], &payload(true), "translation", OLD, true);
    envelope(&posts[1], &payload(true), "agents", OLD, true);
    assert_ne!(posts[0].body["turnId"], posts[1].body["turnId"]);
    assert_eq!(posts.len(), 2);
}

#[tokio::test]
async fn translation_a1_agents_fallback_rejection_does_not_retry_again() {
    assert_eq!(category(), "translation");
    let text = format!("{REJECTED} {}", "é".repeat(510));
    let fixture = Fixture::new(vec![ok(400, REJECTED), ok(400, &text)]);
    assert_eq!(fixture.run(payload(true), true).await.err().unwrap(),
        format!("retryable 400 on {PATH}: {}", text.chars().take(500).collect::<String>()));
    fixture.trace(&["seed", "post", "seed", "post"]);
    let posts = fixture.posts();
    envelope(&posts[0], &payload(true), "translation", OLD, true);
    envelope(&posts[1], &payload(true), "agents", OLD, true);
    assert_ne!(posts[0].body["turnId"], posts[1].body["turnId"]);
}

#[tokio::test]
async fn translation_a2_fallback_401_does_not_compose_with_refresh() {
    assert_eq!(category(), "translation");
    let fixture = Fixture::new(vec![ok(400, REJECTED), ok(401, "expired")]);
    fixture.newer_session_token();
    assert_eq!(fixture.run(payload(true), true).await.err().unwrap(), "unauthorized");
    fixture.trace(&["seed", "post", "seed", "post"]);
    let posts = fixture.posts();
    envelope(&posts[0], &payload(true), "translation", OLD, true);
    envelope(&posts[1], &payload(true), "agents", OLD, true);
    assert_eq!(fixture.transport.authorization.accounts.lock().unwrap()[0].1.bearer, OLD);
}

#[tokio::test]
async fn common_a3_status_classification_never_starts_category_fallback() {
    let castle = include_str!("../../../reference/fixtures/framer-security-refusal.json");
    let upstream = r#"{"request_id":"gen-fixture","error":{"message":"not allowed for this feature"}}"#;
    for (status, body, expected) in [
        (402, REJECTED, "credits-exhausted".into()),
        (403, castle, "risk-refused".into()),
        (403, upstream, format!("upstream 403 on {PATH}: {upstream}")),
        (0, "velocity-paused: fixture; retry in 8s", "account-cooling:9; fixture".into()),
        (429, REJECTED, format!("retryable 429 on {PATH}: {REJECTED}")),
        (500, REJECTED, format!("retryable 500 on {PATH}: {REJECTED}")),
        (400, "different error", format!("retryable 400 on {PATH}: different error")),
    ] {
        let fixture = Fixture::new(vec![ok(status, body)]);
        assert_eq!(fixture.run(payload(true), true).await.err().unwrap(), expected);
        fixture.trace(&["seed", "post"]);
        envelope(&fixture.posts()[0], &payload(true), &category(), OLD, true);
    }
}

#[tokio::test]
async fn common_401_refresh_uses_new_bearer_and_turn_for_response() {
    let fixture = Fixture::new(vec![ok(401, "expired"), ok(200, BODY)]);
    fixture.newer_session_token();
    success(fixture.run(payload(true), true).await.unwrap(), 200, true).await;
    fixture.trace(&["seed", "post", "seed", "post", "queue"]);
    let posts = fixture.posts();
    envelope(&posts[0], &payload(true), &category(), OLD, true);
    envelope(&posts[1], &payload(true), &category(), FRESH, true);
    assert_ne!(posts[0].body["turnId"], posts[1].body["turnId"]);
    assert_eq!(posts.len(), 2);
    let cache = fixture.transport.authorization.accounts.lock().unwrap();
    assert_eq!(cache.len(), 1);
    assert_eq!(cache[0].0, fixture.account);
    assert_eq!(cache[0].1.bearer, FRESH);
}

#[tokio::test]
async fn translation_401_refresh_rejection_does_not_compose_with_category_retry() {
    assert_eq!(category(), "translation");
    let fixture = Fixture::new(vec![ok(401, "expired"), ok(400, REJECTED)]);
    fixture.newer_session_token();
    assert_eq!(fixture.run(payload(true), true).await.err().unwrap(), "category-rejected");
    fixture.trace(&["seed", "post", "seed", "post"]);
    let posts = fixture.posts();
    envelope(&posts[0], &payload(true), "translation", OLD, true);
    envelope(&posts[1], &payload(true), "translation", FRESH, true);
}

#[tokio::test]
async fn common_repeated_401_stops_after_one_refresh() {
    let fixture = Fixture::new(vec![ok(401, "expired"), ok(401, "expired again")]);
    fixture.newer_session_token();
    assert_eq!(fixture.run(payload(true), true).await.err().unwrap(), "unauthorized");
    fixture.trace(&["seed", "post", "seed", "post"]);
    let posts = fixture.posts();
    envelope(&posts[0], &payload(true), &category(), OLD, true);
    envelope(&posts[1], &payload(true), &category(), FRESH, true);
}

#[tokio::test]
#[ignore = "launch with FRAMERIZE_FEATURE_CATEGORY=agents and this exact selector"]
async fn agents_a1_initial_category_rejection_does_not_retry() {
    assert_eq!(category(), "agents");
    let fixture = Fixture::new(vec![ok(400, REJECTED)]);
    assert_eq!(fixture.run(payload(true), true).await.err().unwrap(),
        format!("retryable 400 on {PATH}: {REJECTED}"));
    fixture.trace(&["seed", "post"]);
    envelope(&fixture.posts()[0], &payload(true), "agents", OLD, true);
}

#[tokio::test]
async fn translation_nonstream_fallback_has_two_turns_and_no_refund() {
    assert_eq!(category(), "translation");
    let fixture = Fixture::new(vec![ok(400, REJECTED), ok(200, BODY)]);
    success(fixture.run(payload(false), false).await.unwrap(), 200, false).await;
    fixture.trace(&["seed", "post", "seed", "post", "queue"]);
    let posts = fixture.posts();
    envelope(&posts[0], &payload(false), "translation", OLD, false);
    envelope(&posts[1], &payload(false), "agents", OLD, false);
    assert_ne!(posts[0].body["turnId"], posts[1].body["turnId"]);
}
