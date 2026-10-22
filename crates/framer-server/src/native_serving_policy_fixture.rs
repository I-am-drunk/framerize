//! Synthetic accounts and denied legacy ports. No network or browser exists.
use crate::{AppState, ServingAccountPolicy};
use framer_core::account::{AccountState, ManagedAccount};
use framer_core::convert::{AssembledResponse, AssembledResponses, ChainEvent};
use framer_core::ids::{AccountId, FramerUserId, ProjectId, TeamId};
use framer_core::native_dispatch::{self as port, AttemptMetadata, DispatchError,
    Identity, InvocationProvider, InvocationRun, OwnedInvocation, Policy, Preparation, Route};
use framer_core::pipeline::{InferenceRequest, InferenceResult};
use framer_core::port::*;
use framer_core::workspace::CreditSnapshot;
use futures::{future::BoxFuture, FutureExt};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
use std::time::Duration;
use tokio::{sync::OwnedSemaphorePermit, time::Instant};

pub(super) struct Spy {
    pub forbidden_calls: Mutex<Vec<&'static str>>,
    pub reads: AtomicUsize,
    accounts: Vec<ManagedAccount>,
}

impl Spy {
    fn forbidden(&self, method: &'static str) -> ! {
        self.forbidden_calls.lock().unwrap().push(method);
        panic!("native policy fixture touched forbidden {method}");
    }

    pub fn assert_untouched(&self) {
        self.assert_read_only();
        assert_eq!(self.reads.load(Ordering::SeqCst), 1);
    }
    pub fn assert_read_only(&self) {
        assert!(self.forbidden_calls.lock().unwrap().is_empty());
    }
}

macro_rules! denied {
    ($name:ident <$($lt:lifetime),*> ($($arg:ident: $ty:ty),*) -> $output:ty) => {
        fn $name<'s, $($lt,)* 'a>(&'s self, $($arg: $ty),*) -> BoxFuture<'a, $output>
        where 's: 'a, $($lt: 'a,)* Self: 'a {
            Box::pin(async move { self.forbidden(stringify!($name)) })
        }
    };
}

impl BrowserBridge for Spy {
    denied!(castle_token<'b>(_account: &'b AccountId) -> Result<CastleToken, String>);
    denied!(auth_begin<'b, 'c>(_account: &'b AccountId, _email: &'c str) -> Result<(), String>);
    denied!(auth_complete<'b, 'c>(_account: &'b AccountId, _token: &'c str) -> Result<(String, Value), String>);
    denied!(set_proxy<'b>(_account: &'b AccountId, _proxy: Option<String>) -> Result<(), String>);
    denied!(seed_session<'b, 'c>(_account: &'b AccountId, _cookie: &'c str) -> Result<(), String>);
    denied!(ai_post<'b, 'c, 'd, 'e>(_account: &'b AccountId, _path: &'c str, _bearer: &'d str, _body: &'e Value) -> Result<(u16, Option<String>, String), String>);
    denied!(last_queue_wait_ms<'b>(_account: &'b AccountId) -> u64);
    denied!(rotate_identity<'b>(_account: &'b AccountId) -> Result<(), String>);
}

impl SessionStore for Spy {
    denied!(load<'b>(_user: &'b FramerUserId) -> Result<Option<Session>, String>);
    denied!(save<'b>(_session: &'b Session) -> Result<(), String>);
    denied!(delete<'b>(_user: &'b FramerUserId) -> Result<(), String>);
}

impl StateStore for Spy {
    fn get<'s, 'a>(&'s self, id: AccountId) -> BoxFuture<'a, Result<Option<ManagedAccount>, String>>
    where 's: 'a, Self: 'a {
        Box::pin(async move {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(self.accounts.iter().find(|a| a.id == id).cloned())
        })
    }

    fn list<'s, 'a>(&'s self) -> BoxFuture<'a, Result<Vec<ManagedAccount>, String>>
    where 's: 'a, Self: 'a {
        Box::pin(async move {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(self.accounts.clone())
        })
    }

    denied!(upsert<'b>(_account: &'b ManagedAccount) -> Result<(), String>);
    denied!(remove<>(_id: AccountId) -> Result<(), String>);
}

impl FramerClient for Spy {
    denied!(request_magic_link<'b, 'c>(_email: &'b str, _castle: &'c CastleToken) -> Result<(), String>);
    denied!(complete_magic_link<'b, 'c>(_token: &'b str, _castle: &'c CastleToken) -> Result<Session, String>);
    denied!(me<'b>(_session: &'b Session) -> Result<Value, String>);
    denied!(run_inference<'b, 'c, 'd, 'e, 'f>(_session: &'b Session, _browser: &'c dyn BrowserBridge, _account: &'d AccountId, _project: &'e ProjectId, _req: &'f InferenceRequest) -> Result<InferenceResult, String>);
    denied!(proxy<'b, 'c, 'd, 'e, 'f, 'g>(_session: &'b Session, _browser: &'c dyn BrowserBridge, _account: &'d AccountId, _project: &'e ProjectId, _dialect: &'f str, _payload: Value, _chat: &'g str) -> Result<ProxyResponse, String>);
    denied!(freechat_long<'b, 'c, 'd, 'e, 'f, 'g, 'h>(_session: &'b Session, _browser: &'c dyn BrowserBridge, _account: &'d AccountId, _project: &'e ProjectId, _payload: Value, _cfg: &'f ChainConfig, _chat: &'g str, _events: Option<&'h (dyn Fn(ChainEvent) + Send + Sync)>) -> Result<AssembledResponse, String>);
    denied!(native_long<'b, 'c, 'd, 'e, 'f, 'g, 'h>(_session: &'b Session, _browser: &'c dyn BrowserBridge, _account: &'d AccountId, _project: &'e ProjectId, _payload: Value, _cfg: &'f ChainConfig, _chat: &'g str, _events: Option<&'h (dyn Fn(ChainEvent) + Send + Sync)>) -> Result<AssembledResponses, String>);
    denied!(usage<'b, 'c>(_session: &'b Session, _team: &'c TeamId) -> Result<CreditSnapshot, String>);
    denied!(claim_workspace_grant<'b>(_session: &'b Session) -> Result<bool, String>);
    denied!(create_project<'b, 'c>(_session: &'b Session, _team: &'c TeamId) -> Result<ProjectId, String>);
    denied!(team_projects<'b, 'c>(_session: &'b Session, _team: &'c TeamId) -> Result<Vec<ProjectId>, String>);
    denied!(create_workspace<'b, 'c>(_session: &'b Session, _spec: &'c WorkspaceSpec) -> Result<TeamId, String>);
}

pub(super) fn account(index: u8) -> ManagedAccount {
    ManagedAccount {
        id: AccountId(uuid::Uuid::from_u128(0xC000 + u128::from(index))),
        alias: format!("policy-fixture-{index}"), email: None,
        framer_user_id: FramerUserId(format!("policy-fixture-user-{index}")),
        castle_device_id: format!("policy-fixture-device-{index}"), proxy: None,
        current_team: Some(TeamId(format!("policy-fixture-team-{index}"))),
        current_project: Some(ProjectId(format!("policy-fixture-project-{index}"))),
        extra_workspaces: vec![], state: AccountState::Ready,
        credit: Some(CreditSnapshot { available: 100.0, funds: 1, grants: 1, allocated: 100.0 }),
        consecutive_failures: 0, cooldown_step: 0, cooling_until_ms: None,
    }
}

pub(super) struct PreparationRecord {
    pub identity: Identity, pub route: Route, pub deadline: Instant,
    pub max_total_tokens: u32,
}

pub(super) struct Observation {
    pub identity: Identity, pub route: Route, pub deadline: Instant,
    pub max_total_tokens: u32, pub method: &'static str, pub payload: Value,
}

pub(super) struct ProbeProvider {
    members: HashSet<AccountId>,
    pub membership: Mutex<Vec<AccountId>>,
    pub preparations: Mutex<Vec<PreparationRecord>>,
    pub records: Arc<Mutex<Vec<Observation>>>,
    pub policy: Policy,
}

impl InvocationProvider for ProbeProvider {
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
    fn contains_account(&self, account: AccountId) -> bool {
        self.membership.lock().unwrap().push(account);
        self.members.contains(&account)
    }
    fn prepare(&self, request: Preparation, permit: OwnedSemaphorePermit)
        -> BoxFuture<'_, Result<Box<dyn OwnedInvocation>, DispatchError>> {
        self.preparations.lock().unwrap().push(PreparationRecord {
            identity: request.identity.clone(), route: request.route,
            deadline: request.deadline, max_total_tokens: request.max_total_tokens,
        });
        let call = ProbeCall { request, permit, records: self.records.clone() };
        async move { Ok(Box::new(call) as Box<dyn OwnedInvocation>) }.boxed()
    }
}

struct ProbeCall {
    request: Preparation, permit: OwnedSemaphorePermit,
    records: Arc<Mutex<Vec<Observation>>>,
}
struct NoopCancel;
impl port::Cancellation for NoopCancel { fn cancel(&self) {} }

impl ProbeCall {
    fn run<T: Send + 'static>(self, method: &'static str, payload: Value, value: T)
        -> InvocationRun<T> {
        self.records.lock().unwrap().push(Observation {
            identity: self.request.identity, route: self.request.route,
            deadline: self.request.deadline, max_total_tokens: self.request.max_total_tokens,
            method, payload,
        });
        let permit = self.permit;
        InvocationRun {
            result: async move { Ok(value) }.boxed(),
            cleanup: async move { drop(permit); Ok(()) }.boxed().shared(),
            cancellation: Arc::new(NoopCancel), completed_attempts: None,
        }
    }
}

fn metadata() -> AttemptMetadata {
    AttemptMetadata { status: 200, usage_header: None,
        original_content_type: Some("application/json".into()),
        step_index: 0, elapsed: Duration::ZERO }
}

impl OwnedInvocation for ProbeCall {
    fn raw(self: Box<Self>, payload: Value) -> InvocationRun<port::RawResult> {
        self.run("raw", payload, port::RawResult { body: b"{}".to_vec(), metadata: metadata() })
    }
    fn messages(self: Box<Self>, payload: Value) -> InvocationRun<port::Completed<AssembledResponse>> {
        self.run("messages", payload, port::Completed { value: AssembledResponse {
            id: "policy-message".into(), content: vec![json!({"type":"text","text":"fixture"})],
            stop_reason: "end_turn".into(), stop_sequence: None,
            usage: json!({"input_tokens":1,"output_tokens":1}), input_tokens: 1,
            output_tokens: 1, chunks: 1, metered_chunks: 0,
        }, attempts: vec![metadata()] })
    }
    fn responses(self: Box<Self>, payload: Value) -> InvocationRun<port::Completed<AssembledResponses>> {
        self.run("responses", payload, port::Completed { value: AssembledResponses {
            id: "policy-response".into(), created_at: 1,
            output: vec![json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"fixture"}]})],
            status: "completed".into(), incomplete_details: None,
            usage: json!({"input_tokens":1,"output_tokens":1}), service_tier: Value::Null,
            input_tokens: 1, output_tokens: 1, chunks: 1, metered_chunks: 0,
        }, attempts: vec![metadata()] })
    }
}

pub(super) fn fixture(accounts: Vec<ManagedAccount>, allowed: &[u8], members: &[u8], enabled: bool)
    -> (AppState, Arc<Spy>, Arc<ProbeProvider>) {
    let temporary = std::path::PathBuf::from(std::env::var_os("TMPDIR")
        .expect("the bounded harness must supply TMPDIR"));
    let gate_path = std::path::PathBuf::from(std::env::var_os("FRAMERIZE_GATE_STATE")
        .expect("the bounded harness must isolate FRAMERIZE_GATE_STATE"));
    assert!(temporary.is_absolute() && gate_path.starts_with(&temporary) && gate_path != temporary);
    assert!(!gate_path.components().any(|c| c == std::path::Component::ParentDir));
    let raw = allowed.iter().map(|i| account(*i).id.0.to_string()).collect::<Vec<_>>().join(",");
    let policy = ServingAccountPolicy::parse(Some(&raw)).unwrap();
    let spy = Arc::new(Spy { forbidden_calls: Mutex::new(vec![]),
        reads: AtomicUsize::new(0), accounts });
    let provider = Arc::new(ProbeProvider {
        members: members.iter().map(|i| account(*i).id).collect(),
        membership: Mutex::new(vec![]), preparations: Mutex::new(vec![]),
        records: Arc::new(Mutex::new(vec![])),
        policy: Policy { timeout: Duration::from_secs(2), max_body_bytes: 65536,
            missing_content_type: "application/json".into(), chunk_tokens: 32,
            max_chunks: 3, continuation: None, allow_paid: true, credit_floor: 25.0 },
    });
    let state = AppState {
        operation_ownership: None,
        native_dispatch: if enabled { Some(provider.clone()) } else { None },
        serving_accounts: Arc::new(policy),
        client: spy.clone(), browser: spy.clone(), sessions: spy.clone(), state: spy.clone(),
        gate: Arc::new(framer_core::gate::ConcurrencyGate::with_config(framer_core::gate::GateConfig {
            initial: 1, min: 1, max: 1, recover_ms: 100000, relax_ms: 100000, min_interval_ms: 0,
        })),
        wave: Arc::new(framer_core::wave::WaveGuard::new()),
    };
    (state, spy, provider)
}
