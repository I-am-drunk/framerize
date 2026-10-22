use super::*;
use framer_core::account::{AccountState, ManagedAccount};
use framer_core::ids::{AccountId, FramerUserId, ProjectId, TeamId};
use framer_core::port::*;
use framer_core::pipeline::{InferenceRequest, InferenceResult};
use framer_core::workspace::CreditSnapshot;
use framer_core::native_dispatch::{self as port, InvocationProvider, OwnedInvocation};
use futures::{future::BoxFuture, FutureExt};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::OwnedSemaphorePermit;

pub(super) fn policy() -> Policy {
    Policy { timeout: Duration::from_secs(2), max_body_bytes: 65536,
        missing_content_type: "application/json".into(), chunk_tokens: 32,
        max_chunks: 3, continuation: None, allow_paid: true, credit_floor: 25.0 }
}
pub(super) fn account() -> ManagedAccount {
    ManagedAccount { id: AccountId(uuid::Uuid::from_u128(1)), alias: "fixture".into(),
        email: None, framer_user_id: FramerUserId("fixture-user".into()),
        castle_device_id: "fixture-device".into(), proxy: None,
        current_team: Some(TeamId("fixture-team".into())),
        current_project: Some(ProjectId("fixture-project".into())), extra_workspaces: vec![],
        state: AccountState::Ready, credit: Some(credit()), consecutive_failures: 0,
        cooldown_step: 0, cooling_until_ms: None }
}
fn credit() -> CreditSnapshot {
    CreditSnapshot { available: 100.0, funds: 1, grants: 1, allocated: 100.0 }
}
pub(super) struct Spy {
    pub calls: Mutex<Vec<&'static str>>, pub accounts: Vec<ManagedAccount>,
}
impl Spy { fn called(&self, name: &'static str) { self.calls.lock().unwrap().push(name); } }

#[async_trait::async_trait]
impl BrowserBridge for Spy {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        self.called("castle"); Err("fixture forbids browser".into())
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        self.called("auth_begin"); Err("fixture forbids browser".into())
    }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> {
        self.called("auth_complete"); Err("fixture forbids browser".into())
    }
    async fn seed_session(&self, _: &AccountId, _: &str) -> Result<(), String> {
        self.called("seed_session"); Err("fixture forbids browser".into())
    }
    async fn ai_post(&self, _: &AccountId, _: &str, _: &str, _: &Value)
        -> Result<(u16, Option<String>, String), String> {
        self.called("ai_post"); Err("fixture forbids browser".into())
    }
}

#[async_trait::async_trait]
impl SessionStore for Spy {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        self.called("session_load");
        Ok(Some(Session { active_user: user.clone(), cookie_header: "session=fixture".into() }))
    }
    async fn save(&self, _: &Session) -> Result<(), String> { self.called("save"); Ok(()) }
    async fn delete(&self, _: &FramerUserId) -> Result<(), String> { self.called("delete"); Ok(()) }
}

#[async_trait::async_trait]
impl StateStore for Spy {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        Ok(self.accounts.iter().find(|a| a.id == id).cloned())
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> { Ok(self.accounts.clone()) }
    async fn upsert(&self, _: &ManagedAccount) -> Result<(), String> { self.called("upsert"); Ok(()) }
    async fn remove(&self, _: AccountId) -> Result<(), String> { self.called("remove"); Ok(()) }
}

#[async_trait::async_trait]
impl FramerClient for Spy {
    async fn request_magic_link(&self, _: &str, _: &CastleToken) -> Result<(), String> {
        self.called("magic"); Err("fixture".into())
    }
    async fn complete_magic_link(&self, _: &str, _: &CastleToken) -> Result<Session, String> {
        self.called("complete_magic"); Err("fixture".into())
    }
    async fn me(&self, _: &Session) -> Result<Value, String> { self.called("me"); Err("fixture".into()) }
    async fn run_inference(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: &InferenceRequest) -> Result<InferenceResult, String> {
        self.called("inference"); Err("fixture".into())
    }
    async fn proxy(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: &str, _: Value, _: &str) -> Result<ProxyResponse, String> {
        self.called("proxy");
        Ok(ProxyResponse { status: 200, content_type: "application/json".into(),
            stream: Box::pin(futures::stream::once(async { Ok(bytes::Bytes::from_static(b"{}")) })),
            queue_wait_ms: 0 })
    }
    async fn freechat_long(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(convert::ChainEvent) + Send + Sync)>)
        -> Result<convert::AssembledResponse, String> {
        self.called("messages"); Ok(message_result())
    }
    async fn native_long(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(convert::ChainEvent) + Send + Sync)>)
        -> Result<convert::AssembledResponses, String> {
        self.called("responses"); Ok(response_result())
    }
    async fn usage(&self, _: &Session, _: &TeamId) -> Result<CreditSnapshot, String> {
        self.called("usage"); Ok(credit())
    }
    async fn create_workspace(&self, _: &Session, _: &WorkspaceSpec) -> Result<TeamId, String> {
        self.called("workspace"); Err("fixture".into())
    }
}

fn message_result() -> convert::AssembledResponse {
    convert::AssembledResponse { id: "fixture-message".into(),
        content: vec![serde_json::json!({"type":"text","text":"fixture"})],
        stop_reason: "end_turn".into(), stop_sequence: None,
        usage: serde_json::json!({"input_tokens":2,"output_tokens":1}),
        input_tokens: 2, output_tokens: 1, chunks: 1, metered_chunks: 0 }
}
fn response_result() -> convert::AssembledResponses {
    convert::AssembledResponses { id: "fixture-response".into(), created_at: 1,
        output: vec![serde_json::json!({"type":"message","role":"assistant",
            "content":[{"type":"output_text","text":"fixture"}]})],
        status: "completed".into(), incomplete_details: None,
        usage: serde_json::json!({"input_tokens":2,"output_tokens":1}),
        service_tier: Value::Null, input_tokens: 2, output_tokens: 1,
        chunks: 1, metered_chunks: 0 }
}
fn metadata(status: u16) -> AttemptMetadata {
    AttemptMetadata { status, usage_header: Some("fixture-usage".into()),
        original_content_type: Some("application/json".into()), step_index: 0,
        elapsed: Duration::from_millis(1) }
}
pub(super) struct Observation {
    pub identity: Identity, pub route: Route, pub deadline: Instant,
    pub method: &'static str, pub payload: Value,
}
pub(super) struct FakeProvider {
    pub records: Arc<Mutex<Vec<Observation>>>, pub policy: Policy,
    pub failure: Option<u16>, pub preparations: Arc<Mutex<usize>>,
    pub prepare_failure: Option<FailureKind>,
}
impl InvocationProvider for FakeProvider {
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
    fn contains_account(&self, id: AccountId) -> bool { id == account().id }
    fn prepare(&self, request: Preparation, permit: OwnedSemaphorePermit)
        -> BoxFuture<'_, Result<Box<dyn OwnedInvocation>, DispatchError>> {
        *self.preparations.lock().unwrap() += 1;
        let call = FakeCall { request, permit, records: self.records.clone(), failure: self.failure };
        let failure = self.prepare_failure;
        async move {
            if let Some(kind) = failure { return Err(DispatchError::new(kind)); }
            Ok(Box::new(call) as Box<dyn OwnedInvocation>)
        }.boxed()
    }
}

struct FakeCall {
    request: Preparation, permit: OwnedSemaphorePermit,
    records: Arc<Mutex<Vec<Observation>>>, failure: Option<u16>,
}
struct NoopCancel;
impl port::Cancellation for NoopCancel { fn cancel(&self) {} }
impl FakeCall {
    fn run<T: Send + 'static>(self, method: &'static str, payload: Value, value: T)
        -> InvocationRun<T> {
        self.records.lock().unwrap().push(Observation {
            identity: self.request.identity, route: self.request.route,
            deadline: self.request.deadline, method, payload,
        });
        let permit = self.permit;
        let cleanup = async move { drop(permit); Ok(()) }.boxed().shared();
        let result = match self.failure {
            Some(status) => Err(DispatchError { kind: FailureKind::Upstream,
                upstream: Some(port::UpstreamFailure { metadata: metadata(status),
                    body: b"fixture upstream rejection\nexact bytes".to_vec().into() }), attempts: vec![] }),
            None => Ok(value),
        };
        InvocationRun { result: async move { result }.boxed(), cleanup,
            cancellation: Arc::new(NoopCancel), completed_attempts: None }
    }
}
impl OwnedInvocation for FakeCall {
    fn raw(self: Box<Self>, payload: Value) -> InvocationRun<port::RawResult> {
        self.run("raw", payload, port::RawResult { body: b"{\"fixture\":true}".to_vec(), metadata: metadata(200) })
    }
    fn messages(self: Box<Self>, payload: Value) -> InvocationRun<port::Completed<convert::AssembledResponse>> {
        self.run("messages", payload, port::Completed { value: message_result(), attempts: vec![metadata(200)] })
    }
    fn responses(self: Box<Self>, payload: Value) -> InvocationRun<port::Completed<convert::AssembledResponses>> {
        self.run("responses", payload, port::Completed { value: response_result(), attempts: vec![metadata(200)] })
    }
}

pub(super) fn fixture(enabled: bool, failure: Option<u16>)
    -> (AppState, Arc<Spy>, Arc<FakeProvider>) {
    let gate_path = std::path::PathBuf::from(std::env::var_os("FRAMERIZE_GATE_STATE")
        .expect("the bounded harness must isolate FRAMERIZE_GATE_STATE"));
    let temporary = std::path::PathBuf::from(std::env::var_os("TMPDIR")
        .expect("the bounded harness must supply TMPDIR"));
    assert!(gate_path.starts_with(&temporary) && gate_path != temporary);
    let spy = Arc::new(Spy { calls: Mutex::new(vec![]), accounts: vec![account()] });
    let provider = Arc::new(FakeProvider { records: Arc::new(Mutex::new(vec![])),
        policy: policy(), failure, prepare_failure: None, preparations: Arc::new(Mutex::new(0)) });
    let state = AppState {
        operation_ownership: None,
        native_dispatch: if enabled { Some(provider.clone()) } else { None },
        serving_accounts: Arc::new(crate::ServingAccountPolicy::default()),
        client: spy.clone(), browser: spy.clone(), sessions: spy.clone(), state: spy.clone(),
        gate: Arc::new(framer_core::gate::ConcurrencyGate::with_config(framer_core::gate::GateConfig {
            initial: 2, min: 1, max: 2, recover_ms: 100000, relax_ms: 100000, min_interval_ms: 0,
        })),
        wave: Arc::new(framer_core::wave::WaveGuard::new()),
    };
    (state, spy, provider)
}
