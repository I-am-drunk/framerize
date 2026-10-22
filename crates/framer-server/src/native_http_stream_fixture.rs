//! Bounded in-memory streams and cleanup ownership. No socket or browser.
use super::*;
use crate::{AppState, ServingAccountPolicy};
use framer_core::account::ManagedAccount;
use framer_core::gate::{ConcurrencyGate, GateConfig};
use framer_core::ids::{AccountId, FramerUserId, ProjectId, TeamId};
use framer_core::native_dispatch::*;
use std::collections::VecDeque;
use std::sync::{atomic::{AtomicUsize, Ordering}, Mutex};
use tokio::sync::{mpsc, oneshot};
use tower::ServiceExt;

#[path = "native_dispatch_fixture.rs"]
mod legacy;

pub const BOUND: Duration = Duration::from_secs(2);

pub fn limits(heartbeat: Option<Duration>) -> StreamingPolicy {
    StreamingPolicy {
        raw: RawStreamLimits { chunk_bytes: 64, retained_bytes: 256,
            total_bytes: 4096, header_bytes: 4096 },
        composed: ComposedStreamLimits { frame_bytes: 512, retained_bytes: 2048,
            retained_frames: 4, state_bytes: 65536, json_bytes: 8192,
            source_chunk_bytes: 64, header_bytes: 4096, total_bytes: 65536,
            terminal_bytes: 512 },
        heartbeat,
    }
}

pub fn headers(status: u16, content_type: &str, usage: Option<&str>) -> Arc<RawStreamHeaders> {
    Arc::new(RawStreamHeaders { status, content_type: Arc::from(content_type),
        original_content_type: Some(Arc::from(content_type)),
        usage_header: usage.map(Arc::from), elapsed: Duration::from_millis(7) })
}

#[derive(Default)]
pub struct Probe {
    pub cancelled: AtomicUsize,
    pub body_polls: AtomicUsize,
    pub body_drops: AtomicUsize,
    pub emitted: AtomicUsize,
    cleanup: Mutex<Option<Cleanup>>,
}
impl Cancellation for Probe {
    fn cancel(&self) { self.cancelled.fetch_add(1, Ordering::SeqCst); }
}
impl Probe {
    pub fn cancellations(&self) -> usize { self.cancelled.load(Ordering::SeqCst) }
    pub fn cleanup(&self) -> Cleanup {
        self.cleanup.lock().unwrap().as_ref().expect("invocation started").clone()
    }
}

struct Body {
    receive: mpsc::Receiver<Result<bytes::Bytes, FailureKind>>, probe: Arc<Probe>,
}
impl Stream for Body {
    type Item = Result<bytes::Bytes, FailureKind>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.probe.body_polls.fetch_add(1, Ordering::SeqCst);
        let result = self.receive.poll_recv(cx);
        if let Poll::Ready(Some(Ok(bytes))) = &result {
            self.probe.emitted.fetch_add(bytes.len(), Ordering::SeqCst);
        }
        result
    }
}
impl Drop for Body {
    fn drop(&mut self) { self.probe.body_drops.fetch_add(1, Ordering::SeqCst); }
}

pub struct Control {
    head: Option<oneshot::Sender<Result<(), FailureKind>>>,
    frames: Option<mpsc::Sender<Result<bytes::Bytes, FailureKind>>>,
    finished: Option<oneshot::Sender<Result<(), FailureKind>>>,
    cleanup: Option<oneshot::Sender<Result<(), FailureKind>>>,
    pub probe: Arc<Probe>,
    pub headers: Arc<RawStreamHeaders>,
}
struct Plan {
    head: oneshot::Receiver<Result<(), FailureKind>>,
    frames: mpsc::Receiver<Result<bytes::Bytes, FailureKind>>,
    finished: oneshot::Receiver<Result<(), FailureKind>>,
    cleanup: oneshot::Receiver<Result<(), FailureKind>>,
    probe: Arc<Probe>,
    headers: Arc<RawStreamHeaders>,
    content_type: &'static str,
}
impl Control {
    pub fn open(&mut self) { self.head.take().unwrap().send(Ok(())).unwrap(); }
    pub fn reject(&mut self, kind: FailureKind) {
        self.head.take().unwrap().send(Err(kind)).unwrap();
    }
    pub fn push(&self, bytes: &'static [u8]) {
        self.frames.as_ref().unwrap().try_send(Ok(bytes::Bytes::from_static(bytes))).unwrap();
    }
    pub fn fail_body(&self, kind: FailureKind) {
        self.frames.as_ref().unwrap().try_send(Err(kind)).unwrap();
    }
    pub fn eof(&mut self) { self.frames.take(); }
    pub fn finish(&mut self, result: Result<(), FailureKind>) {
        self.finished.take().unwrap().send(result).unwrap();
    }
    pub async fn release(&mut self) {
        self.cleanup.take().unwrap().send(Ok(())).unwrap();
        assert_eq!(tokio::time::timeout(BOUND, self.probe.cleanup()).await.unwrap(), Ok(()));
        self.probe.cleanup.lock().unwrap().take();
    }
}

fn plan(headers: Arc<RawStreamHeaders>, content_type: &'static str) -> (Control, Plan) {
    let (head_tx, head) = oneshot::channel();
    let (frames_tx, frames) = mpsc::channel(4);
    let (finished_tx, finished) = oneshot::channel();
    let (cleanup_tx, cleanup) = oneshot::channel();
    let probe = Arc::new(Probe::default());
    (Control { head: Some(head_tx), frames: Some(frames_tx),
        finished: Some(finished_tx), cleanup: Some(cleanup_tx),
        probe: probe.clone(), headers: headers.clone() },
     Plan { head, frames, finished, cleanup, probe, headers, content_type })
}

impl Plan {
    fn raw(self, permit: OwnedSemaphorePermit) -> RawStreamRun {
        let cleanup = async move {
            let result = self.cleanup.await.unwrap_or(Err(FailureKind::Cleanup));
            drop(permit);
            result
        }.boxed().shared();
        *self.probe.cleanup.lock().unwrap() = Some(cleanup.clone());
        let source: RawStreamBody = Box::pin(Body {
            receive: self.frames, probe: self.probe.clone(),
        });
        let headers = self.headers.clone();
        let cancellation = self.probe.clone();
        let result = async move {
            self.head.await.unwrap_or(Err(FailureKind::Cancelled)).map_err(DispatchError::new)?;
            Ok(RawStreamHead { headers, body: source })
        }.boxed();
        let finished = async move {
            RawStreamCompletion {
                result: self.finished.await.unwrap_or(Err(FailureKind::Cancelled)),
                headers: Some(self.headers),
                emitted_bytes: self.probe.emitted.load(Ordering::SeqCst),
                elapsed: Duration::from_millis(9),
            }
        }.boxed().shared();
        RawStreamRun { response: InvocationRun { result, cleanup, cancellation,
            completed_attempts: None }, finished }
    }
}

#[derive(Clone)]
pub struct Record {
    pub identity: Identity,
    pub route: Route,
    pub deadline: Instant,
    pub max_total_tokens: u32,
    pub method: &'static str,
    pub payload: Value,
    pub raw: Option<RawStreamLimits>,
    pub composed: Option<ComposedStreamOptions>,
    pub client_model: Option<String>,
}
struct Provider {
    accounts: Vec<AccountId>,
    policy: Policy,
    plans: Mutex<VecDeque<(AccountId, Plan)>>,
    records: Arc<Mutex<Vec<Record>>>,
}
struct Call {
    request: Preparation,
    permit: OwnedSemaphorePermit,
    plan: Plan,
    records: Arc<Mutex<Vec<Record>>>,
    client_model: Option<String>,
}
impl InvocationProvider for Provider {
    fn supports_pool_admission(&self) -> bool { true }
    fn prepare_in_pool(&self, request: Preparation, permit: OwnedSemaphorePermit,
        admission: framer_core::native_dispatch::PoolAdmission)
        -> BoxFuture<'_, Result<Box<dyn OwnedInvocation>, DispatchError>> {
        assert_eq!(admission.account(), request.identity.account);
        let prepared = self.prepare(request, permit);
        async move {
            Ok(crate::native_dispatch::pool_fixture::hold(prepared.await?, admission))
        }.boxed()
    }

    fn policy(&self) -> Policy { self.policy.clone() }
    fn contains_account(&self, account: AccountId) -> bool { self.accounts.contains(&account) }
    fn prepare(&self, request: Preparation, permit: OwnedSemaphorePermit)
        -> BoxFuture<'_, Result<Box<dyn OwnedInvocation>, DispatchError>> {
        let plan = {
            let mut plans = self.plans.lock().unwrap();
            let position = plans.iter().position(|(id, _)| *id == request.identity.account)
                .expect("unexpected account or extra invocation");
            plans.remove(position).unwrap().1
        };
        let call = Call { request, permit, plan, records: self.records.clone(), client_model: None };
        async move { Ok(Box::new(call) as Box<dyn OwnedInvocation>) }.boxed()
    }
}
impl Call {
    fn record(&self, method: &'static str, payload: Value,
        raw: Option<RawStreamLimits>, composed: Option<ComposedStreamOptions>) {
        self.records.lock().unwrap().push(Record {
            identity: self.request.identity.clone(), route: self.request.route,
            deadline: self.request.deadline, max_total_tokens: self.request.max_total_tokens,
            method, payload, raw, composed, client_model: self.client_model.clone(),
        });
    }
}

impl OwnedInvocation for Call {
    fn composed_stream_for_client(mut self: Box<Self>, payload: Value,
        options: ComposedStreamOptions, client_model: String) -> ComposedStreamRun {
        self.client_model = Some(client_model);
        self.composed_stream(payload, options)
    }
    fn raw_stream(self: Box<Self>, payload: Value, limits: RawStreamLimits) -> RawStreamRun {
        self.record("raw_stream", payload, Some(limits), None);
        let Call { plan, permit, .. } = *self;
        plan.raw(permit)
    }
    fn composed_stream(self: Box<Self>, payload: Value, options: ComposedStreamOptions)
        -> ComposedStreamRun {
        self.record("composed_stream", payload, None, Some(options));
        let Call { plan, permit, .. } = *self;
        let content_type = plan.content_type;
        let raw = plan.raw(permit);
        let InvocationRun { result, cleanup, cancellation, completed_attempts } = raw.response;
        let result = result.map(move |result| result.map(|head| ComposedStreamHead {
            first_attempt: head.headers, content_type, body: head.body,
        })).boxed();
        let finished = raw.finished.map(|value| ComposedStreamCompletion {
            result: value.result, emitted_bytes: value.emitted_bytes, elapsed: value.elapsed,
            attempts: value.headers.into_iter().map(|headers| ComposedAttemptMetadata {
                headers, response_id: Some(Arc::from("fixture-response")),
                usage: None, step_index: 0, elapsed: value.elapsed,
            }).collect::<Vec<_>>().into(),
        }).boxed().shared();
        ComposedStreamRun { response: InvocationRun {
            result, cleanup, cancellation, completed_attempts,
        }, finished }
    }
    fn raw(self: Box<Self>, _: Value) -> InvocationRun<RawResult> {
        panic!("stream must not use the collected raw path")
    }
    fn messages(self: Box<Self>, _: Value) -> InvocationRun<Completed<convert::AssembledResponse>> {
        panic!("stream must not use collected Messages")
    }
    fn responses(self: Box<Self>, _: Value) -> InvocationRun<Completed<convert::AssembledResponses>> {
        panic!("stream must not use collected Responses")
    }
}

pub struct Fixture {
    pub state: AppState,
    pub accounts: Vec<ManagedAccount>,
    pub streaming: StreamingPolicy,
    provider: Arc<Provider>,
    spy: Arc<legacy::Spy>,
}
impl Fixture {
    pub fn new(count: usize, timeout: Duration, heartbeat: Option<Duration>) -> Self {
        let temporary = std::path::PathBuf::from(std::env::var_os("TMPDIR").expect("private TMPDIR"));
        let gate = std::path::PathBuf::from(std::env::var_os("FRAMERIZE_GATE_STATE")
            .expect("private gate state"));
        assert!(gate.starts_with(&temporary) && gate != temporary);
        let accounts: Vec<_> = (0..count).map(|index| {
            let mut account = legacy::account();
            let id = uuid::Uuid::new_v4();
            account.id = AccountId(id);
            account.alias = format!("http-fixture-{index}");
            account.framer_user_id = FramerUserId(format!("{id}-user"));
            account.current_team = Some(TeamId(format!("{id}-team")));
            account.current_project = Some(ProjectId(format!("{id}-project")));
            account.castle_device_id = format!("{id}-device");
            account
        }).collect();
        let spy = Arc::new(legacy::Spy { calls: Mutex::new(vec![]), accounts: accounts.clone() });
        let mut policy = legacy::policy();
        policy.timeout = timeout;
        let provider = Arc::new(Provider { accounts: accounts.iter().map(|a| a.id).collect(),
            policy, plans: Mutex::new(VecDeque::new()), records: Arc::new(Mutex::new(vec![])) });
        let streaming = limits(heartbeat);
        let state = AppState {
        operation_ownership: None,
            native_dispatch: Some(with_native_streaming(provider.clone(), streaming).unwrap()),
            client: spy.clone(), browser: spy.clone(), sessions: spy.clone(), state: spy.clone(),
            serving_accounts: Arc::new(ServingAccountPolicy::default()),
            gate: Arc::new(ConcurrencyGate::with_config(GateConfig {
                initial: 1, min: 1, max: 1, recover_ms: 60000, relax_ms: 60000, min_interval_ms: 0,
            })),
            wave: Arc::new(framer_core::wave::WaveGuard::new()),
        };
        Self { state, accounts, streaming, provider, spy }
    }
}

impl Fixture {
    pub fn enqueue(&self, index: usize, headers: Arc<RawStreamHeaders>,
        content_type: &'static str) -> Control {
        let (control, plan) = plan(headers, content_type);
        self.provider.plans.lock().unwrap().push_back((self.accounts[index].id, plan));
        control
    }
    pub fn records(&self) -> Vec<Record> { self.provider.records.lock().unwrap().clone() }
    pub fn assert_legacy_unused(&self) { assert!(self.spy.calls.lock().unwrap().is_empty()); }
    pub async fn slot_free(&self, index: usize, model: &str) -> bool {
        let key = format!("{}:{}", self.accounts[index].current_team.as_ref().unwrap().0,
            convert::to_openrouter_id(model));
        let slot = self.state.gate.acquire_now(&key).await;
        let free = slot.is_some();
        drop(slot);
        free
    }
    pub async fn request(&self, path: &str, payload: Value, index: usize,
        session: &str, paid: bool) -> Response {
        let mut request = axum::http::Request::builder().method("POST").uri(path)
            .header("content-type", "application/json")
            .header("x-framerize-account", self.accounts[index].id.0.to_string())
            .header("x-framerize-session", session);
        if paid { request = request.header("x-framerize-mode", "paid"); }
        tokio::time::timeout(BOUND, crate::router(self.state.clone(), "/fixture-no-web")
            .oneshot(request.body(axum::body::Body::from(payload.to_string())).unwrap()))
            .await.expect("bounded in-process request").unwrap()
    }
}

pub fn responses() -> Value {
    serde_json::json!({"model":"gpt-4.1", "stream":true, "max_output_tokens":32,
        "input":[{"role":"user","content":[{"type":"input_text","text":"fixture"}]}],
        "fixture_extension":{"opaque":true}})
}

pub fn kind(error: &axum::Error) -> FailureKind {
    let mut current: &(dyn std::error::Error + 'static) = error;
    loop {
        if let Some(value) = current.downcast_ref::<HttpStreamError>() { return value.0; }
        current = current.source().expect("native stream error in the source chain");
    }
}
