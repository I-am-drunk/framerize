//! Offline real-router support. Binaries are supplied and hashed by main.
use framer_core::{account::ManagedAccount, ids::*, port::*,
    pipeline::{InferenceRequest, InferenceResult}, workspace::CreditSnapshot};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot};
#[path = "permit_lifetime_fixture.rs"]
mod base;
pub const BOUND: Duration = Duration::from_secs(3);
pub fn memory_points() -> Value {
    let mut children = std::collections::BTreeSet::new();
    for task in std::fs::read_dir("/proc/self/task").unwrap() {
        let path = task.unwrap().path().join("children");
        if let Ok(text) = std::fs::read_to_string(path) {
            children.extend(text.split_whitespace().map(|pid| pid.parse::<u32>().unwrap()));
        }
    }
    json!({"kernel":memory_point(std::process::id()),
        "children":children.into_iter().map(memory_point).collect::<Vec<_>>()})
}
fn memory_point(pid: u32) -> Value {
    let read = || -> std::io::Result<Value> {
        let before = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let status = std::fs::read_to_string(format!("/proc/{pid}/status"))?;
        let after = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let start = |stat: &str| stat.rsplit_once(')').unwrap().1.split_whitespace()
            .nth(19).unwrap().parse::<u64>().unwrap();
        if start(&before) != start(&after) { return Err(std::io::Error::other("process changed")); }
        let field = |name: &str| status.lines().find_map(|line| line.strip_prefix(name))
            .and_then(|value| value.split_whitespace().next()).and_then(|v| v.parse::<u64>().ok());
        Ok(json!({"pid":pid, "start_ticks":start(&after), "rss_kib":field("VmRSS:"),
            "vmsize_kib":field("VmSize:"), "threads":field("Threads:")}))
    };
    read().unwrap_or_else(|_| json!({"pid":pid,"unavailable":true}))
}
pub struct LogDirectory(std::path::PathBuf);
impl LogDirectory {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!("policy-router-log-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        framer_core::observability::init_path(&path);
        Self(path)
    }
    pub fn records(&self) -> Vec<Value> {
        let files: Vec<_> = std::fs::read_dir(self.0.join(".logs")).unwrap().collect();
        assert_eq!(files.len(), 1);
        std::fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap()
            .lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }
    pub async fn wait_diagnostic(&self, kind: &str) {
        tokio::time::timeout(BOUND, async {
            loop {
                for file in std::fs::read_dir(self.0.join(".logs")).unwrap() {
                    let text = std::fs::read_to_string(file.unwrap().path()).unwrap();
                    // A concurrent append may leave the last line incomplete.
                    if text.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok())
                        .any(|record| record["diagnostic"]["kind"] == kind) { return; }
                }
                tokio::task::yield_now().await;
            }
        }).await.expect("diagnostic did not reach the private log");
    }
}
impl Drop for LogDirectory {
    fn drop(&mut self) { std::fs::remove_dir_all(&self.0).unwrap(); }
}
pub struct RealPeer {
    pub process: crate::policy_process::PolicyProcess,
    pub input: mpsc::Receiver<framer_worker_wire::frame::Decoded<framer_worker_wire::message::Message>>,
    control: framer_worker_wire::frame::WriteSlot,
}
impl RealPeer {
    pub async fn new() -> Self {
        use crate::policy_process::{Dispatch, Executable, PolicyProcess};
        use framer_worker_wire::{message::Message, WireError};
        let (send, input) = mpsc::channel(8);
        let dispatch: Dispatch = Arc::new(move |frame| send.try_send(frame).map_err(|_| WireError::Protocol));
        let (path, sha256) = executable("WORKER");
        let process = tokio::time::timeout(BOUND,
            PolicyProcess::spawn(Executable { path, sha256 }, 1, dispatch)).await.unwrap().unwrap();
        let control = process.outbox.register(1).unwrap();
        let mut peer = Self { process, input, control };
        assert!(matches!(peer.next().await.message, Message::Hello { wire: 1, .. }));
        peer.send(Message::Ready { generation: 1 }).await;
        peer
    }
    pub async fn next(&mut self) -> framer_worker_wire::frame::Decoded<framer_worker_wire::message::Message> {
        tokio::time::timeout(BOUND, self.input.recv()).await.unwrap().unwrap()
    }
    pub async fn send(&self, message: framer_worker_wire::message::Message) {
        tokio::time::timeout(BOUND, self.control.send(&message)).await.unwrap().unwrap();
    }
    pub async fn begin(&mut self) {
        use framer_worker_wire::message::{Message, Text, Mode, Limits, Part, Segment};
        self.send(Message::Start { op: 7, logical_id: Text::new("fixed".into()).unwrap(),
            model: Text::new("gpt-6-astra".into()).unwrap(), mode: Mode::Sse,
            output: crate::response_output::WORKER_OUTPUT }).await;
        assert!(matches!(self.next().await.message, Message::Head { op: 7, status: 200, .. }));
        self.send(Message::Invoke { op: 7, n: 1, category: Text::new("translation".into()).unwrap(),
            limits: Limits { chunk_tokens: 600, max_chunks: 1, max_total_tokens: None,
                metered_fallback: false, continue_prompt: Text::new("continue".into()).unwrap() },
            request: Part { offset: 0, last: true,
                bytes: Segment::new(serde_json::to_vec(&payload()).unwrap()).unwrap() } }).await;
    }
    pub async fn close(mut self) {
        self.process.request_drain();
        let exit = tokio::time::timeout(BOUND, self.process.join()).await.unwrap().unwrap();
        assert!(exit.reaped);
    }
}
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
#[derive(Default)]
pub struct EncoderBytes(pub Vec<u8>);
impl std::io::Write for EncoderBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(bytes); Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}
impl framer_core::convert::ResponseWriter for EncoderBytes {
    fn begin_event(&mut self, _: u64, _: bool) -> std::io::Result<()> { Ok(()) }
    fn end_event(&mut self) -> std::io::Result<()> { Ok(()) }
}
pub fn expected_tool_stream(reply: &ForwardReply) -> Vec<u8> {
    use framer_core::convert::{AssembledResponses, ResponseEncoder};
    let body: Value = serde_json::from_slice(&reply.body).unwrap();
    let assembled = AssembledResponses { id: "upstream".into(), created_at: 123,
        output: body["output"].as_array().unwrap().clone(), status: "completed".into(),
        incomplete_details: None, usage: body["usage"].clone(), service_tier: Value::Null,
        input_tokens: 1, output_tokens: 1, chunks: 1, metered_chunks: 0 };
    let mut encoder = ResponseEncoder::new("gpt-6-astra", "fixed");
    let mut bytes = EncoderBytes::default();
    encoder.open(&mut bytes).unwrap();
    bytes.0.clear(); // Wall-clock opening is checked separately by the router trace.
    encoder.push_items(&mut bytes, &assembled.output, false).unwrap();
    encoder.finish(&mut bytes, &assembled).unwrap();
    bytes.0
}
pub fn tool_reply(arguments: &str) -> ForwardReply {
    let mut result = reply("");
    let mut body: Value = serde_json::from_slice(&result.body).unwrap();
    body["output"] = json!([{"type":"function_call", "id":"fc_fixed",
        "call_id":"call_Exact-Café_42", "name":"write", "arguments":arguments,
        "status":"completed"}]);
    result.body = serde_json::to_vec(&body).unwrap().into();
    result
}
pub fn trace_tool(text: &str, arguments: &str) {
    let events: Vec<Value> = text.lines().filter_map(|line| line.strip_prefix("data: "))
        .filter(|line| *line != "[DONE]").map(|line| serde_json::from_str(line).unwrap()).collect();
    let id = events[0]["response"]["id"].as_str().unwrap();
    assert_eq!(events[0]["type"], "response.created");
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["sequence_number"], index as u64);
        if let Some(response) = event.get("response") { assert_eq!(response["id"], id); }
    }
    let terminal: Vec<_> = events.iter().filter(|e| e["type"] == "response.completed").collect();
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0]["response"]["output"][0], json!({"type":"function_call",
        "id":"fc_fixed", "call_id":"call_Exact-Café_42", "name":"write",
        "arguments":arguments, "status":"completed"}));
    let delta: String = events.iter().filter(|e| e["type"] == "response.function_call_arguments.delta")
        .map(|e| e["delta"].as_str().unwrap()).collect();
    assert_eq!(delta, arguments);
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
    assert_eq!(item["id"], format!("msg_{}_1", id.trim_start_matches("resp_")));
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
    pub fn with_client_browser(client: Arc<dyn FramerClient>, browser: Arc<dyn BrowserBridge>) -> Self {
        let mut base = base::Fixture::new();
        let (_, receiver) = mpsc::channel(8);
        let runtime = base.state.runtime_mut().unwrap();
        runtime.client = client;
        runtime.browser = browser;
        let state = crate::operation_ownership::with_owned_operations(base.state, 64).unwrap();
        let state = crate::policy_generations::with_policy_workers(state).unwrap();
        Self { state, accounts: base.accounts, calls: receiver,
            _base_calls: base.calls, _loads: base.loads }
    }
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
pub async fn profile(worker: bool) {
    use futures::StreamExt;
    let mut f = Fixture::new();
    if worker { let generation = f.stage().await; f.manager().activate(generation).unwrap(); }
    let before = memory_points();
    assert_eq!(before["children"].as_array().unwrap().len(), usize::from(worker));
    let input = payload();
    let request_bytes = input.to_string().len();
    let expected = "profile café ".repeat(8192);
    let reply = reply(&expected);
    let reply_bytes = reply.body.len();
    let started = std::time::Instant::now();
    let response = f.post(Some(0), input).await;
    let head_us = started.elapsed().as_micros();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let call = f.next().await;
    let forward_us = started.elapsed().as_micros();
    call.reply.send(Ok(reply)).ok().unwrap();
    call.joined.await.unwrap();
    let mut body = response.into_body().into_data_stream();
    let mut output = Vec::new();
    let (mut content_us, mut terminal_us) = (None, None);
    while let Some(bytes) = tokio::time::timeout(BOUND, body.next()).await.unwrap() {
        let bytes = bytes.unwrap();
        let text = String::from_utf8_lossy(&bytes);
        if text.contains("event: response.output_item.added\n") {
            content_us.get_or_insert(started.elapsed().as_micros());
        }
        if text.contains("event: response.completed\n") {
            terminal_us.get_or_insert(started.elapsed().as_micros());
        }
        output.extend_from_slice(&bytes);
        assert!(output.len() <= 2 * 1024 * 1024);
    }
    let eof_us = started.elapsed().as_micros();
    drop(body);
    finish_profile(f, &output, &expected, before, worker, request_bytes, reply_bytes,
        [head_us, forward_us, content_us.unwrap(), terminal_us.unwrap(), eof_us]).await;
}

#[allow(clippy::too_many_arguments)]
async fn finish_profile(f: Fixture, output: &[u8], expected: &str, before: Value,
    worker: bool, request_bytes: usize, reply_bytes: usize, milestones: [u128; 5]) {
    trace(std::str::from_utf8(output).unwrap(), expected);
    let after_eof = memory_points();
    let peaks = f.state.response_output.peak_output_counts();
    assert!(peaks.0 > 0 && peaks.0 <= 32768);
    assert!(peaks.1 > 0 && peaks.1 <= 2);
    let state = f.state.clone();
    f.finish().await;
    assert_eq!(state.response_output.counts(), (0, 0, 0));
    assert_eq!(state.response_output.output_counts(), (0, 0));
    let after_join = memory_points();
    assert!(after_join["children"].as_array().unwrap().is_empty());
    println!("{}", json!({"profile":"s3w_router", "mode":if worker {"worker"} else {"legacy0"},
        "head_us":milestones[0], "forward_us":milestones[1], "first_content_us":milestones[2],
        "terminal_start_us":milestones[3], "eof_us":milestones[4],
        "peak_output_bytes":peaks.0, "peak_output_slots":peaks.1,
        "final_leases":0, "final_jobs":0, "final_bytes":0,
        "final_output_bytes":0, "final_output_slots":0,
        "request_json_bytes":request_bytes, "reply_bytes":reply_bytes,
        "data_wire_reserve":framer_worker_wire::credit::DATA_WIRE_RESERVE,
        "before":before, "after_eof":after_eof, "after_join":after_join,
        "limits":"RSS point samples; unpaused aggregate milestones; no socket or exact total-memory claim"}));
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
