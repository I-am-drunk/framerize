use super::*;
use async_trait::async_trait;
use framer_core::{FramerUserId, port::{CastleToken, ChainConfig, FramerClient}};
use serde_json::json;
use std::{collections::VecDeque, sync::{Arc, atomic::{AtomicUsize, Ordering}}};

type Reply = (u16, Option<String>, String);
#[derive(Debug)]
struct Post { account: AccountId, bearer: String, body: Value }
struct Browser {
    replies: Mutex<VecDeque<Reply>>,
    posts: Mutex<Vec<Post>>,
    seeds: Mutex<Vec<(AccountId, String)>>,
    order: Mutex<Vec<&'static str>>,
    queue_ms: u64,
}

#[async_trait]
impl BrowserBridge for Browser {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> { panic!("unexpected mint") }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> { panic!("unexpected login") }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> { panic!("unexpected login") }
    async fn seed_session(&self, id: &AccountId, cookie: &str) -> Result<(), String> {
        self.seeds.lock().unwrap().push((*id, cookie.into()));
        self.order.lock().unwrap().push("seed");
        Ok(())
    }
    async fn ai_post(&self, id: &AccountId, path: &str, bearer: &str, body: &Value)
        -> Result<Reply, String>
    {
        self.order.lock().unwrap().push(match path {
            "/ai/v3/responses/" => "inference", "/ai/v3/refund/" => "refund",
            _ => panic!("unexpected path"),
        });
        self.posts.lock().unwrap().push(Post { account: *id, bearer: bearer.into(), body: body.clone() });
        Ok(self.replies.lock().unwrap().pop_front().expect("unexpected extra post"))
    }
    async fn last_queue_wait_ms(&self, _: &AccountId) -> u64 {
        self.order.lock().unwrap().push("queue");
        self.queue_ms
    }
}

struct Fixture { client: crate::HttpFramerClient, account: AccountId, session: Session, browser: Browser }

impl Fixture {
    fn new(replies: Vec<Reply>) -> Self {
        assert!(std::env::var("FRAMERIZE_JWT_RELAY_URL").unwrap_or_default().is_empty());
        let transport = Transport::control_for_loopback("127.0.0.1:9".parse().unwrap(),
            crate::session_auth::Egress::Direct).unwrap();
        let account = AccountId(uuid::Uuid::new_v4());
        let session = Session { active_user: FramerUserId("fixture-user".into()),
            cookie_header: "fixture-cookie".into() };
        transport.authorization.accounts.lock().unwrap().push((account,
            AiToken::for_session(&session, "fixture-bearer".into(), i64::MAX)));
        Self { client: crate::HttpFramerClient { transport }, account, session,
            browser: Browser { replies: Mutex::new(replies.into()), posts: Mutex::new(Vec::new()),
                seeds: Mutex::new(Vec::new()), order: Mutex::new(Vec::new()), queue_ms: 4321 } }
    }
}

fn success() -> Reply {
    (200, None, json!({"id":"fixture-response", "status":"completed", "output":[],
        "usage":{"input_tokens":1, "output_tokens":0}}).to_string())
}
fn payload() -> Value { json!({"model":"gpt-6-astra", "input":"fixture"}) }

#[derive(Clone, Copy)]
enum FaultAt { Seed, Forward, Refund }
struct FaultingBrowser<'a> { inner: &'a Browser, at: FaultAt, message: &'a str }
#[async_trait]
impl BrowserBridge for FaultingBrowser<'_> {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> { panic!("unexpected mint") }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> { panic!("unexpected login") }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> { panic!("unexpected login") }
    async fn seed_session(&self, id: &AccountId, cookie: &str) -> Result<(), String> {
        if matches!(self.at, FaultAt::Seed) { return Err(self.message.into()); }
        self.inner.seed_session(id, cookie).await
    }
    async fn ai_post(&self, id: &AccountId, path: &str, bearer: &str, body: &Value) -> Result<Reply, String> {
        if matches!((self.at, path), (FaultAt::Forward, "/ai/v3/responses/")
            | (FaultAt::Refund, "/ai/v3/refund/")) { return Err(self.message.into()); }
        self.inner.ai_post(id, path, bearer, body).await
    }
    async fn last_queue_wait_ms(&self, id: &AccountId) -> u64 { self.inner.last_queue_wait_ms(id).await }
}

fn typed_error(result: Result<ForwardReply, PolicyError>) -> ForwardError {
    match result {
        Err(PolicyError::Forward(error)) => error,
        Err(other) => panic!("unexpected error kind: {other}"),
        Ok(_) => panic!("expected an upstream failure"),
    }
}
#[tokio::test]
async fn borrowed_factory_construction_performs_no_external_work() {
    let f = Fixture::new(vec![]);
    let project = ProjectId("fixture-project".into());
    let cfg = ChainConfig::default();
    let forward = f.client.bind_responses(&f.session, &f.browser, &f.account,
        &project, &cfg, "fixture-chat").unwrap();
    assert_eq!(forward.category(), proxy::category_from_env());
    assert!(f.browser.seeds.lock().unwrap().is_empty());
    assert!(f.browser.posts.lock().unwrap().is_empty());
    assert!(f.browser.order.lock().unwrap().is_empty());
}

#[tokio::test]
async fn borrowed_factory_keeps_post_refund_queue_time_and_binding() {
    let f = Fixture::new(vec![(206, None, "data: fixture\n\n".into()),
        (200, None, "{\"granted\":false}".into())]);
    let project = ProjectId("fixture-project".into());
    let cfg = ChainConfig::default();
    let forward = f.client.bind_responses(&f.session, &f.browser, &f.account,
        &project, &cfg, "fixture-chat").unwrap();
    let reply = forward.forward_typed("translate", json!({"stream":true}), 0).await.unwrap();
    assert_eq!(reply.body.as_ref(), b"data: fixture\n\n");
    assert_eq!((reply.status, reply.queue_wait_ms), (206, 4321));
    assert_eq!(reply.content_type, "text/event-stream");
    assert_eq!(*f.browser.order.lock().unwrap(), ["seed", "inference", "refund", "queue"]);
    let posts = f.browser.posts.lock().unwrap();
    assert!(posts.iter().all(|post| post.account == f.account && post.bearer == "fixture-bearer"
        && post.body["projectId"] == project.0 && post.body["sessionId"] == "fixture-chat"));
}

#[tokio::test]
async fn identical_http_and_bridge_error_text_keeps_distinct_origin() {
    let body = r#"{"error":{"message":"fixture-private","type":"odd / λ"}}"#;
    let raw = format!("retryable 429 on /ai/v3/responses/: {body}");
    let f = Fixture::new(vec![(429, None, body.into())]);
    let project = ProjectId("fixture-project".into());
    let cfg = ChainConfig::default();
    let forward = f.client.bind_responses(&f.session, &f.browser, &f.account,
        &project, &cfg, "fixture-chat").unwrap();
    let http = typed_error(forward.forward_typed("translate", payload(), 0).await);
    let broken = FaultingBrowser { inner: &f.browser, at: FaultAt::Forward, message: &raw };
    let local = f.client.bind_responses(&f.session, &broken, &f.account,
        &project, &cfg, "fixture-chat").unwrap();
    let local = typed_error(local.forward_typed("translate", payload(), 0).await);
    assert_eq!(http.to_string(), raw);
    assert_eq!(local.to_string(), raw);
    assert_eq!(http.class, local.class);
    assert_eq!(http.origin, framer_core::port::ErrorOrigin::Http);
    assert_eq!(local.origin, framer_core::port::ErrorOrigin::Local);
}

#[tokio::test]
async fn borrowed_factory_preserves_absent_and_false_refund_callbacks() {
    for supplied in [false, true] {
        let f = Fixture::new(vec![success()]);
        let browser = FaultingBrowser { inner: &f.browser, at: FaultAt::Refund,
            message: "retryable 429 on /fixture: private refund failure" };
        let project = ProjectId("fixture-project".into());
        let mut cfg = ChainConfig::default();
        if supplied { cfg.cancelled = Some(Arc::new(|| false)); }
        let forward = f.client.bind_responses(&f.session, &browser, &f.account,
            &project, &cfg, "fixture-chat").unwrap();
        let result = forward.forward_typed("translate", json!({"stream":true}), 0).await;
        assert_eq!(result.is_err(), supplied);
        if supplied {
            let error = typed_error(result);
            assert_eq!(error.origin, framer_core::port::ErrorOrigin::Local);
            assert_eq!(error.to_string(), browser.message);
        }
        assert_eq!(f.browser.order.lock().unwrap().last() == Some(&"queue"), !supplied);
    }
}

#[tokio::test]
async fn borrowed_factory_cancellation_prevents_dispatch_and_seed_errors_stay_local() {
    for cancelled in [false, true] {
        let f = Fixture::new(vec![]);
        let browser = FaultingBrowser { inner: &f.browser, at: FaultAt::Seed,
            message: "retryable 429 on /fixture: private seed failure" };
        let project = ProjectId("fixture-project".into());
        let cfg = ChainConfig { cancelled: Some(Arc::new(move || cancelled)),
            ..ChainConfig::default() };
        let forward = f.client.bind_responses(&f.session, &browser, &f.account,
            &project, &cfg, "fixture-chat").unwrap();
        let error = typed_error(forward.forward_typed("translate", payload(), 0).await);
        assert_eq!(error.origin, framer_core::port::ErrorOrigin::Local);
        assert_eq!(error.to_string(), if cancelled { "client cancelled" } else { browser.message });
        assert!(f.browser.posts.lock().unwrap().is_empty());
        assert!(f.browser.order.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn typed_category_fallback_keeps_fresh_snapshot_and_http_origin() {
    assert_eq!(proxy::category_from_env(), "translation");
    let f = Fixture::new(vec![(400, None, "not allowed for this feature".into()),
        (429, None, "busy".into()), success()]);
    let project = ProjectId("fixture-project".into());
    let cfg = ChainConfig::default();
    let forward = f.client.bind_responses(&f.session, &f.browser, &f.account,
        &project, &cfg, "fixture-chat").unwrap();
    let error = typed_error(forward.forward_typed("translate", payload(), 0).await);
    assert_eq!(error.origin, framer_core::port::ErrorOrigin::Http);
    assert!(error.is_saturated());
    forward.forward_typed("translate", payload(), 1).await.unwrap();
    let posts = f.browser.posts.lock().unwrap();
    assert_eq!(posts.len(), 3);
    for (post, category) in posts.iter().zip(["translation", "agents", "translation"]) {
        assert_eq!(post.body["featureCategory"], category);
    }
    assert_eq!(forward.category(), "translation");
}

#[tokio::test]
async fn borrowed_factory_rebinds_credentials_project_chat_and_saturation() {
    let f = Fixture::new(vec![(429, None, "busy".into()), (429, None, "busy".into())]);
    let counts = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
    for i in 0..2 {
        let account = AccountId(uuid::Uuid::from_u128(300 + i as u128));
        let session = Session { active_user: FramerUserId(format!("factory-user-{i}")),
            cookie_header: format!("factory-cookie-{i}") };
        let bearer = format!("factory-bearer-{i}");
        f.client.transport.authorization.accounts.lock().unwrap().push((account,
            AiToken::for_session(&session, bearer.clone(), i64::MAX)));
        let counter = counts[i].clone();
        let cfg = ChainConfig { on_429: Some(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })), ..ChainConfig::default() };
        let project = ProjectId(format!("factory-project-{i}"));
        let chat = format!("factory-chat-{i}");
        let forward = f.client.bind_responses(&session, &f.browser, &account,
            &project, &cfg, &chat).unwrap();
        let error = typed_error(forward.forward_typed("translate", payload(), 0).await);
        assert!(error.is_saturated());
        assert_eq!(counts[i].load(Ordering::SeqCst), 0);
        forward.saturated().await.unwrap();
        assert_eq!(counts[0].load(Ordering::SeqCst), 1);
        assert_eq!(counts[1].load(Ordering::SeqCst), i);
        let posts = f.browser.posts.lock().unwrap();
        assert_eq!(posts.len(), i + 1);
        assert_eq!(posts[i].account, account);
        assert_eq!(posts[i].bearer, bearer);
        assert_eq!(posts[i].body["projectId"], project.0);
        assert_eq!(posts[i].body["sessionId"], chat);
        let seeds = f.browser.seeds.lock().unwrap();
        assert_eq!(seeds[i], (account, session.cookie_header.clone()));
    }
}

#[tokio::test(start_paused = true)]
async fn category_fallback_does_not_change_the_next_chunks_snapshot() {
    assert_eq!(proxy::category_from_env(), "translation");
    let partial = json!({"id":"fixture-partial", "status":"incomplete",
        "incomplete_details":{"reason":"max_output_tokens"},
        "output":[{"type":"message", "role":"assistant",
            "content":[{"type":"output_text", "text":"part"}]}],
        "usage":{"input_tokens":1, "output_tokens":16}});
    let f = Fixture::new(vec![(400, None, "not allowed for this feature".into()),
        (200, None, partial.to_string()), success()]);
    let result = f.client.native_long(&f.session, &f.browser, &f.account,
        &ProjectId("fixture-project".into()), payload(),
        &ChainConfig { max_chunks: 2, ..ChainConfig::default() }, "fixture-chat", None)
        .await.unwrap();
    assert_eq!(result.chunks, 2);
    let posts = f.browser.posts.lock().unwrap();
    assert_eq!(posts.len(), 3);
    for (post, (category, index)) in posts.iter().zip([
        ("translation", 0), ("agents", 0), ("translation", 1)]) {
        assert_eq!(post.body["featureCategory"], category);
        assert_eq!(post.body["step"], "translate");
        assert_eq!(post.body["stepIndex"], index);
    }
    assert!(f.browser.replies.lock().unwrap().is_empty());
}

async fn launch_category(expected: &str, step: &str) {
    assert_eq!(proxy::category_from_env(), expected, "wrong per-process launch category");
    let f = Fixture::new(vec![success()]);
    let result = f.client.native_long(&f.session, &f.browser, &f.account,
        &ProjectId("fixture-project".into()), payload(), &ChainConfig::default(),
        "fixture-chat", None).await.unwrap();
    assert_eq!(result.id, "fixture-response");
    let posts = f.browser.posts.lock().unwrap();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].body["step"], step);
    assert_eq!(posts[0].body["featureCategory"], expected);
    assert!(f.browser.replies.lock().unwrap().is_empty());
}

#[tokio::test]
async fn translation_launch_category_drives_native_step_and_envelope() {
    launch_category("translation", "translate").await;
}

#[tokio::test]
#[ignore = "requires FRAMERIZE_FEATURE_CATEGORY=agents in a separate process"]
async fn agents_launch_category_drives_native_step_and_envelope() {
    launch_category("agents", "message").await;
}

#[tokio::test]
async fn bound_category_controls_policy_and_envelope_without_another_env_read() {
    assert_eq!(proxy::category_from_env(), "translation");
    let f = Fixture::new(vec![success()]);
    let project = ProjectId("fixture-project".into());
    let mut forward = ResponsesForward::new(&f.client.transport,
        &f.client.transport.authorization.accounts, &f.session, &f.browser,
        &f.account, &project, "fixture-chat", None, None);
    assert_eq!(forward.category(), "translation");
    // Model a captured value different from the ambient setting. No env mutation.
    forward.category = "agents".into();
    let result = framer_protocol::responses::complete_long(&forward, payload(),
        &ChainConfig::default().limits(), None).await.unwrap();
    assert_eq!(result.id, "fixture-response");
    let posts = f.browser.posts.lock().unwrap();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].body["featureCategory"], "agents");
    assert_eq!(posts[0].body["featureName"], "agents");
    assert_eq!(posts[0].body["step"], "message");
    assert!(f.browser.replies.lock().unwrap().is_empty());
}

#[tokio::test]
async fn nonzero_queue_wait_and_body_survive_the_post_refund_lookup() {
    let f = Fixture::new(vec![(206, None, "data: fixture\n\n".into()),
        (200, None, "{\"granted\":false}".into())]);
    let project = ProjectId("fixture-project".into());
    let forward = ResponsesForward::new(&f.client.transport,
        &f.client.transport.authorization.accounts, &f.session, &f.browser,
        &f.account, &project, "fixture-chat", None, None);
    let result = forward.forward("translate", json!({"model":"fixture", "stream":true}), 0)
        .await.unwrap();
    assert_eq!(result.status, 206);
    assert_eq!(result.content_type, "text/event-stream");
    assert_eq!(result.body.as_ref(), b"data: fixture\n\n");
    assert_eq!(result.queue_wait_ms, 4321);
    assert_eq!(*f.browser.order.lock().unwrap(), ["seed", "inference", "refund", "queue"]);
    assert!(f.browser.replies.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn native_long_rebinds_account_session_project_chat_and_saturation() {
    let f = Fixture::new(vec![(429, None, "busy".into()), success(),
        (429, None, "busy".into()), success()]);
    let counts = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
    for i in 0..2 {
        let account = AccountId(uuid::Uuid::from_u128(100 + i as u128));
        let session = Session { active_user: FramerUserId(format!("fixture-user-{i}")),
            cookie_header: format!("fixture-cookie-{i}") };
        let bearer = format!("fixture-bearer-{i}");
        f.client.transport.authorization.accounts.lock().unwrap().push((account,
            AiToken::for_session(&session, bearer.clone(), i64::MAX)));
        let counter = counts[i].clone();
        let cfg = ChainConfig { on_429: Some(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })), ..ChainConfig::default() };
        let project = ProjectId(format!("fixture-project-{i}"));
        let chat = format!("fixture-chat-{i}");
        let result = f.client.native_long(&session, &f.browser, &account, &project,
            payload(), &cfg, &chat, None).await.unwrap();
        assert_eq!(result.id, "fixture-response");
        assert_eq!(counts[0].load(Ordering::SeqCst), 1);
        assert_eq!(counts[1].load(Ordering::SeqCst), i);
        let posts = f.browser.posts.lock().unwrap();
        assert_eq!(posts.len(), 2 * (i + 1));
        for post in &posts[2 * i..] {
            assert_eq!(post.account, account);
            assert_eq!(post.bearer, bearer);
            assert_eq!(post.body["projectId"], project.0);
            assert_eq!(post.body["sessionId"], chat);
        }
        let seeds = f.browser.seeds.lock().unwrap();
        assert_eq!(seeds.len(), 2 * (i + 1));
        assert_eq!(&seeds[2 * i..], &[(account, session.cookie_header.clone()),
            (account, session.cookie_header.clone())]);
    }
    assert!(f.browser.replies.lock().unwrap().is_empty());
}

struct AwaitedSink { calls: AtomicUsize, reject: bool }
#[async_trait]
impl framer_core::port::ResponsesSink for AwaitedSink {
    async fn emit(&self, chunk: framer_core::port::ValidatedChunk)
        -> Result<framer_core::port::ValidatedChunk, framer_core::port::OutputError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.reject { Err(framer_core::port::OutputError::Encoding) } else { Ok(chunk) }
    }
}
#[tokio::test(start_paused = true)]
async fn awaited_entry_keeps_authorized_binding_and_output_error_type() {
    let f = Fixture::new(vec![success()]);
    let sink = AwaitedSink { calls: AtomicUsize::new(0), reject: true };
    let result = f.client.native_long_with_sink(&f.session, &f.browser, &f.account,
        &ProjectId("fixture-project".into()), payload(), &ChainConfig::default(),
        "fixture-chat", &sink).await;
    assert!(matches!(result, Err(framer_core::port::ResponseError::Output(
        framer_core::port::OutputError::Encoding))));
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.browser.posts.lock().unwrap().len(), 1);
}
