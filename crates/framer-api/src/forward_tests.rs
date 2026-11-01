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
    let limit = "too many concurrent AI requests for this workspace";
    let f = Fixture::new(vec![(429, None, limit.into()), success(),
        (429, None, limit.into()), success()]);
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
