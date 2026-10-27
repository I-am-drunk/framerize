//! Offline real-router support. Binaries are supplied and hashed by main.
use framer_core::{account::ManagedAccount, ids::*, port::*,
    pipeline::{InferenceRequest, InferenceResult}, workspace::CreditSnapshot};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot};
#[path = "permit_lifetime_fixture.rs"]
mod base;
pub const BOUND: Duration = Duration::from_secs(3);
pub fn executable(kind: &str) -> (std::path::PathBuf, [u8; 32]) {
    let path = std::env::var_os(format!("FRAMERIZE_TEST_POLICY_{kind}_PATH"))
        .expect("main must supply an explicit built executable path");
    let hash = std::env::var(format!("FRAMERIZE_TEST_POLICY_{kind}_SHA256"))
        .expect("main must supply its executable SHA256");
    assert_eq!(hash.len(), 64);
    let mut bytes = [0; 32];
    for (index, value) in bytes.iter_mut().enumerate() {
        *value = u8::from_str_radix(&hash[index * 2..index * 2 + 2], 16).unwrap();
    }
    let path = std::path::PathBuf::from(path);
    assert!(path.is_absolute());
    (path, bytes)
}
pub fn payload() -> Value { json!({"model":"gpt-6-astra", "stream":true, "input":"fixture"}) }
pub fn reply(text: &str) -> ForwardReply {
    ForwardReply { status: 200, content_type: "application/json".into(), queue_wait_ms: 73,
        body: serde_json::to_vec(&json!({"id":"upstream", "created_at":123, "status":"completed",
            "output":[{"type":"message", "id":"msg_exact", "role":"assistant", "status":"completed",
                "content":[{"type":"output_text", "text":text, "annotations":[]}]}],
            "usage":{"input_tokens":1,"output_tokens":1}})).unwrap().into() }
}
pub async fn consume(response: axum::response::Response) -> String {
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = tokio::time::timeout(BOUND,
        crate::response_output::consume_body(response.into_body(), 2 * 1024 * 1024)).await.unwrap().unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}
pub fn trace(text: &str, expected: &str) -> String {
    let events: Vec<Value> = text.lines().filter_map(|line| line.strip_prefix("data: "))
        .filter(|line| *line != "[DONE]").map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(events.first().unwrap()["type"], "response.created");
    let id = events[0]["response"]["id"].as_str().unwrap().to_owned();
    let created = events[0]["response"]["created_at"].as_u64().unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    assert!((1_600_000_000..=now + 1).contains(&created));
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["sequence_number"].as_u64(), Some(index as u64));
        if let Some(response) = event.get("response") { assert_eq!(response["id"], id); }
    }
    let terminal: Vec<_> = events.iter().filter(|event| event["type"] == "response.completed").collect();
    assert_eq!(terminal.len(), 1);
    let item = &terminal[0]["response"]["output"][0];
    assert_eq!(item["id"], "msg_exact");
    assert_eq!(item["content"][0]["text"], expected);
    let delta: String = events.iter().filter(|event| event["type"] == "response.output_text.delta")
        .map(|event| event["delta"].as_str().unwrap()).collect();
    assert_eq!(delta, expected);
    id
}
pub struct Fixture {
    pub state: crate::AppState, pub accounts: Vec<ManagedAccount>, pub calls: mpsc::Receiver<Call>,
    _base_calls: mpsc::UnboundedReceiver<base::Call>, _loads: mpsc::UnboundedReceiver<FramerUserId>,
}
impl Fixture {
    pub fn new() -> Self {
        let mut base = base::Fixture::new();
        let (calls, receiver) = mpsc::channel(8);
        let runtime = base.state.runtime_mut().unwrap();
        runtime.client = Arc::new(Client { inner: runtime.client.clone(), calls });
        let state = crate::operation_ownership::with_owned_operations(base.state, 64).unwrap();
        let state = crate::policy_generations::with_policy_workers(state).unwrap();
        Self { state, accounts: base.accounts, calls: receiver,
            _base_calls: base.calls, _loads: base.loads }
    }
    pub fn manager(&self) -> Arc<crate::policy_generations::GenerationManager> {
        self.state.policy_generations().unwrap().clone()
    }
    pub async fn stage(&self) -> u64 {
        self.stage_kind("WORKER").await
    }
    pub async fn stage_kind(&self, kind: &str) -> u64 {
        let (path, hash) = executable(kind);
        tokio::time::timeout(BOUND, self.manager().stage(path, hash)).await.unwrap().unwrap()
    }
    pub async fn wait_failed(&self, generation: u64) {
        tokio::time::timeout(BOUND, async {
            while self.manager().snapshot().unwrap().iter()
                .any(|slot| slot.generation == generation && slot.alive) {
                tokio::task::yield_now().await;
            }
        }).await.expect("fault peer did not terminate");
    }
    pub async fn finish_fenced(self) {
        use crate::policy_generations::{GenerationError, GenerationPhase};
        self.settle().await;
        assert!(!self.state.operation_ownership.as_ref().unwrap().accepting_requests());
        assert_eq!(tokio::time::timeout(BOUND, self.manager().shutdown(true)).await.unwrap(),
            Err(GenerationError::Unconfirmed));
        let slots = self.manager().snapshot().unwrap();
        assert!(!slots.is_empty());
        assert!(slots.iter().all(|slot| slot.pins == 0 && !slot.alive
            && slot.unconfirmed && slot.phase == GenerationPhase::ReapedUnconfirmed));
        assert_eq!(self.state.response_output.counts(), (0, 0, 0));
    }
    pub async fn next(&mut self) -> Call {
        tokio::time::timeout(BOUND, self.calls.recv()).await.unwrap().unwrap()
    }
    pub async fn post(&self, account: Option<usize>, payload: Value) -> axum::response::Response {
        use tower::ServiceExt;
        let mut request = axum::http::Request::builder().method("POST")
            .uri("/openai/v1/responses").header("content-type", "application/json")
            .header("x-framerize-session", "worker-fixture-session");
        if let Some(account) = account {
            request = request.header("x-framerize-account", self.accounts[account].id.0.to_string());
        }
        crate::router(self.state.clone(), "/nonexistent-worker-fixture-web")
            .oneshot(request.body(axum::body::Body::from(payload.to_string())).unwrap())
            .await.unwrap()
    }
    pub async fn settle(&self) {
        tokio::time::timeout(BOUND, async {
            loop {
                if self.manager().snapshot().unwrap().iter().all(|slot| slot.pins == 0)
                    && self.state.response_output.counts() == (0, 0, 0) { break; }
                tokio::task::yield_now().await;
            }
        }).await.expect("last operation owner did not settle");
    }
    pub async fn finish(self) {
        self.settle().await;
        assert!(self.state.operation_ownership.as_ref().unwrap().accepting_requests());
        tokio::time::timeout(BOUND, self.manager().shutdown(false)).await.unwrap().unwrap();
        assert!(self.manager().snapshot().unwrap().is_empty());
    }
}
pub struct Call {
    pub account: AccountId, pub payload: Value, pub step: String, pub chunk: u32,
    pub reply: oneshot::Sender<Result<ForwardReply, PolicyError>>,
    pub joined: oneshot::Receiver<()>,
}
struct Joined(Option<oneshot::Sender<()>>);
impl Drop for Joined { fn drop(&mut self) { if let Some(send) = self.0.take() { let _ = send.send(()); } } }
struct Client { inner: Arc<dyn FramerClient>, calls: mpsc::Sender<Call> }
struct Binding<'a> { client: &'a Client, account: AccountId, cfg: &'a ChainConfig }
#[async_trait::async_trait]
impl FramerClient for Client {
    fn bind_responses<'a>(&'a self, _session: &'a Session, _browser: &'a dyn BrowserBridge,
        account: &'a AccountId, _project: &'a ProjectId, cfg: &'a ChainConfig, _chat: &'a str)
        -> Option<Box<dyn AuthorizedForward + 'a>> {
        Some(Box::new(Binding { client: self, account: *account, cfg }))
    }
    async fn native_long_with_sink(&self, session: &Session, browser: &dyn BrowserBridge,
        account: &AccountId, project: &ProjectId, payload: Value, cfg: &ChainConfig,
        chat: &str, sink: &dyn ResponsesSink) -> ResponseResult {
        let binding = self.bind_responses(session, browser, account, project, cfg, chat).unwrap();
        framer_protocol::responses::complete_long_with_sink(binding.as_ref(), payload,
            &cfg.limits(), sink).await
    }
    async fn request_magic_link(&self, email: &str, castle: &CastleToken) -> Result<(), String> {
        self.inner.request_magic_link(email, castle).await
    }
    async fn complete_magic_link(&self, token: &str, castle: &CastleToken) -> Result<Session, String> {
        self.inner.complete_magic_link(token, castle).await
    }
    async fn me(&self, session: &Session) -> Result<Value, String> { self.inner.me(session).await }
    async fn run_inference(&self, session: &Session, browser: &dyn BrowserBridge,
        account: &AccountId, project: &ProjectId, request: &InferenceRequest) -> Result<InferenceResult, String> {
        self.inner.run_inference(session, browser, account, project, request).await
    }
    async fn proxy(&self, session: &Session, browser: &dyn BrowserBridge,
        account: &AccountId, project: &ProjectId, dialect: &str, payload: Value,
        chat: &str) -> Result<ProxyResponse, String> {
        self.inner.proxy(session, browser, account, project, dialect, payload, chat).await
    }
    async fn freechat_long(&self, session: &Session, browser: &dyn BrowserBridge,
        account: &AccountId, project: &ProjectId, payload: Value, cfg: &ChainConfig,
        chat: &str, events: Option<&(dyn Fn(framer_core::convert::ChainEvent) + Send + Sync)>)
        -> Result<framer_core::convert::AssembledResponse, String> {
        self.inner.freechat_long(session, browser, account, project, payload, cfg, chat, events).await
    }
    async fn native_long(&self, session: &Session, browser: &dyn BrowserBridge,
        account: &AccountId, project: &ProjectId, payload: Value, cfg: &ChainConfig,
        chat: &str, events: Option<&(dyn Fn(framer_core::convert::ChainEvent) + Send + Sync)>)
        -> Result<framer_core::convert::AssembledResponses, String> {
        self.inner.native_long(session, browser, account, project, payload, cfg, chat, events).await
    }
    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        self.inner.usage(session, team).await
    }
    async fn create_workspace(&self, session: &Session, spec: &WorkspaceSpec) -> Result<TeamId, String> {
        self.inner.create_workspace(session, spec).await
    }
}
impl ChainControl for Binding<'_> {
    fn is_cancelled(&self) -> bool { self.cfg.is_cancelled() }
    fn on_429(&self) { if let Some(callback) = &self.cfg.on_429 { callback(); } }
}
#[async_trait::async_trait]
impl AuthorizedForward for Binding<'_> {
    fn category(&self) -> &str { "translation" }
    async fn forward(&self, step: &str, payload: Value, chunk: u32) -> Result<ForwardReply, String> {
        self.forward_typed(step, payload, chunk).await.map_err(|error| error.to_string())
    }
    async fn forward_typed(&self, step: &str, payload: Value, chunk: u32) -> Result<ForwardReply, PolicyError> {
        let (reply, result) = oneshot::channel();
        let (joined, observed) = oneshot::channel();
        let _joined = Joined(Some(joined));
        self.client.calls.send(Call { account: self.account, payload, step: step.into(), chunk,
            reply, joined: observed }).await.expect("fixture receiver");
        result.await.expect("explicit fixture reply")
    }
}
