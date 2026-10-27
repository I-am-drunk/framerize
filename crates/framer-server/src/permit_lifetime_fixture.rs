//! In-memory ports for real handler and gate ownership tests.
use crate::{AppState, ServingAccountPolicy};
use framer_core::account::{AccountState, ManagedAccount};
use framer_core::convert::{AssembledResponse, AssembledResponses, ChainEvent};
use framer_core::gate::{ConcurrencyGate, GateConfig};
use framer_core::ids::{AccountId, FramerUserId, ProjectId, TeamId};
use framer_core::pipeline::{InferenceRequest, InferenceResult};
use framer_core::port::*;
use framer_core::workspace::CreditSnapshot;
use futures::future::BoxFuture;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

pub enum Reply {
    Proxy(ProxyResponse),
    Messages(AssembledResponse),
    Responses(AssembledResponses),
}
pub struct Call {
    pub kind: &'static str,
    pub config: Option<ChainConfig>,
    pub account: AccountId,
    pub reply: oneshot::Sender<Result<Reply, String>>,
    pub dropped: oneshot::Receiver<()>,
}
struct CallDrop(Option<oneshot::Sender<()>>);
impl Drop for CallDrop {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() { let _ = tx.send(()); }
    }
}
struct Ports {
    records: Mutex<Vec<ManagedAccount>>,
    calls: mpsc::UnboundedSender<Call>,
    loads: mpsc::UnboundedSender<FramerUserId>,
}

impl Ports {
    fn unexpected(&self, port: &str) -> ! { panic!("unexpected {port} I/O") }

    async fn invoke(&self, kind: &'static str, account: &AccountId, config: Option<&ChainConfig>) -> Result<Reply, String> {
        let (reply, receive) = oneshot::channel();
        let (dropped, observed) = oneshot::channel();
        let _drop = CallDrop(Some(dropped));
        self.calls.send(Call { kind, config: config.cloned(), account: account.clone(), reply, dropped: observed })
            .map_err(|_| "fixture calls closed".to_string())?;
        receive.await.map_err(|_| "fixture reply closed".to_string())?
    }
}

macro_rules! denied {
    ($port:literal, $name:ident <$($lt:lifetime),*> ($($arg:ident: $ty:ty),*) -> $output:ty) => {
        fn $name<'s, $($lt,)* 'a>(&'s self, $($arg: $ty),*) -> BoxFuture<'a, $output>
        where 's: 'a, $($lt: 'a,)* Self: 'a {
            Box::pin(async move { self.unexpected($port) })
        }
    };
}

impl BrowserBridge for Ports {
    denied!("browser", castle_token<'b>(_account: &'b AccountId) -> Result<CastleToken, String>);
    denied!("browser", auth_begin<'b, 'c>(_account: &'b AccountId, _email: &'c str) -> Result<(), String>);
    denied!("browser", auth_complete<'b, 'c>(_account: &'b AccountId, _token: &'c str) -> Result<(String, Value), String>);
    denied!("browser", set_proxy<'b>(_account: &'b AccountId, _proxy: Option<String>) -> Result<(), String>);
    denied!("browser", seed_session<'b, 'c>(_account: &'b AccountId, _cookie: &'c str) -> Result<(), String>);
    denied!("browser", ai_post<'b, 'c, 'd, 'e>(_account: &'b AccountId, _path: &'c str, _bearer: &'d str, _body: &'e Value) -> Result<(u16, Option<String>, String), String>);
    denied!("browser", last_queue_wait_ms<'b>(_account: &'b AccountId) -> u64);
    denied!("browser", rotate_identity<'b>(_account: &'b AccountId) -> Result<(), String>);
}


impl FramerClient for Ports {
    fn native_long_with_sink<'s, 'b, 'c, 'd, 'e, 'f, 'g, 'h, 'a>(
        &'s self, _session: &'b Session, _browser: &'c dyn BrowserBridge,
        account: &'d AccountId, _project: &'e ProjectId, _payload: Value,
        _cfg: &'f ChainConfig, _chat: &'g str, sink: &'h dyn ResponsesSink,
    ) -> BoxFuture<'a, ResponseResult>
    where 's: 'a, 'b: 'a, 'c: 'a, 'd: 'a, 'e: 'a, 'f: 'a, 'g: 'a, 'h: 'a, Self: 'a {
        Box::pin(async move {
            match self.invoke("responses", account, Some(_cfg)).await? {
                Reply::Responses(value) => emit_assembled(value, sink).await,
                _ => panic!("wrong fixture reply"),
            }
        })
    }
    denied!("client", request_magic_link<'b, 'c>(_email: &'b str, _castle: &'c CastleToken) -> Result<(), String>);
    denied!("client", complete_magic_link<'b, 'c>(_token: &'b str, _castle: &'c CastleToken) -> Result<Session, String>);
    denied!("client", run_inference<'b, 'c, 'd, 'e, 'f>(_session: &'b Session, _browser: &'c dyn BrowserBridge, _account: &'d AccountId, _project: &'e ProjectId, _req: &'f InferenceRequest) -> Result<InferenceResult, String>);
    denied!("client", claim_workspace_grant<'b>(_session: &'b Session) -> Result<bool, String>);
    denied!("client", create_project<'b, 'c>(_session: &'b Session, _team: &'c TeamId) -> Result<ProjectId, String>);
    denied!("client", team_projects<'b, 'c>(_session: &'b Session, _team: &'c TeamId) -> Result<Vec<ProjectId>, String>);
    denied!("client", create_workspace<'b, 'c>(_session: &'b Session, _spec: &'c WorkspaceSpec) -> Result<TeamId, String>);
    denied!("client", me<'b>(_session: &'b Session) -> Result<Value, String>);
    fn usage<'s, 'b, 'c, 'a>(&'s self, _session: &'b Session, _team: &'c TeamId) -> BoxFuture<'a, Result<CreditSnapshot, String>>
    where 's: 'a, 'b: 'a, 'c: 'a, Self: 'a {
        Box::pin(async { Ok(CreditSnapshot { available: 100.0, funds: 1, grants: 1, allocated: 100.0 }) })
    }
    fn proxy<'s, 'b, 'c, 'd, 'e, 'f, 'g, 'a>(&'s self, _session: &'b Session, _browser: &'c dyn BrowserBridge, account: &'d AccountId, _project: &'e ProjectId, _dialect: &'f str, _payload: Value, _chat: &'g str) -> BoxFuture<'a, Result<ProxyResponse, String>>
    where 's: 'a, 'b: 'a, 'c: 'a, 'd: 'a, 'e: 'a, 'f: 'a, 'g: 'a, Self: 'a {
        Box::pin(async move { match self.invoke("proxy", account, None).await? {
            Reply::Proxy(value) => Ok(value), _ => panic!("wrong fixture reply"),
        } })
    }
    fn freechat_long<'s, 'b, 'c, 'd, 'e, 'f, 'g, 'h, 'a>(&'s self, _session: &'b Session, _browser: &'c dyn BrowserBridge, account: &'d AccountId, _project: &'e ProjectId, _payload: Value, _cfg: &'f ChainConfig, _chat: &'g str, events: Option<&'h (dyn Fn(ChainEvent) + Send + Sync)>) -> BoxFuture<'a, Result<AssembledResponse, String>>
    where 's: 'a, 'b: 'a, 'c: 'a, 'd: 'a, 'e: 'a, 'f: 'a, 'g: 'a, 'h: 'a, Self: 'a {
        Box::pin(async move { match self.invoke("messages", account, Some(_cfg)).await? {
            Reply::Messages(value) => { if let Some(cb) = events { cb(ChainEvent::Chunk(value.content.clone())); } Ok(value) }, _ => panic!("wrong fixture reply"),
        } })
    }
    fn native_long<'s, 'b, 'c, 'd, 'e, 'f, 'g, 'h, 'a>(&'s self, _session: &'b Session, _browser: &'c dyn BrowserBridge, account: &'d AccountId, _project: &'e ProjectId, _payload: Value, _cfg: &'f ChainConfig, _chat: &'g str, events: Option<&'h (dyn Fn(ChainEvent) + Send + Sync)>) -> BoxFuture<'a, Result<AssembledResponses, String>>
    where 's: 'a, 'b: 'a, 'c: 'a, 'd: 'a, 'e: 'a, 'f: 'a, 'g: 'a, 'h: 'a, Self: 'a {
        Box::pin(async move { match self.invoke("responses", account, Some(_cfg)).await? {
            Reply::Responses(value) => { if let Some(cb) = events { cb(ChainEvent::ResponsesItems { items: value.output.clone(), continues_text: false }); } Ok(value) }, _ => panic!("wrong fixture reply"),
        } })
    }
}
impl SessionStore for Ports {
    fn load<'s, 'b, 'a>(&'s self, user: &'b FramerUserId) -> BoxFuture<'a, Result<Option<Session>, String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            self.loads.send(user.clone()).unwrap();
            Ok(Some(Session { active_user: user.clone(), cookie_header: "session=fixture-only".into() }))
        })
    }
    denied!("session", save<'b>(_session: &'b Session) -> Result<(), String>);
    denied!("session", delete<'b>(_user: &'b FramerUserId) -> Result<(), String>);
}
impl StateStore for Ports {
    fn get<'s, 'a>(&'s self, id: AccountId) -> BoxFuture<'a, Result<Option<ManagedAccount>, String>>
    where 's: 'a, Self: 'a {
        Box::pin(async move { Ok(self.records.lock().unwrap().iter().find(|a| a.id == id).cloned()) })
    }
    fn list<'s, 'a>(&'s self) -> BoxFuture<'a, Result<Vec<ManagedAccount>, String>>
    where 's: 'a, Self: 'a {
        Box::pin(async move { Ok(self.records.lock().unwrap().clone()) })
    }
    fn upsert<'s, 'b, 'a>(&'s self, account: &'b ManagedAccount) -> BoxFuture<'a, Result<(), String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            let mut records = self.records.lock().unwrap();
            *records.iter_mut().find(|a| a.id == account.id).unwrap() = account.clone();
            Ok(())
        })
    }
    denied!("state", remove<>(_id: AccountId) -> Result<(), String>);
}

pub struct Fixture {
    pub state: AppState,
    pub accounts: Vec<ManagedAccount>,
    pub calls: mpsc::UnboundedReceiver<Call>,
    pub loads: mpsc::UnboundedReceiver<FramerUserId>,
}
impl Fixture {
    pub fn new() -> Self {
        let prefix = uuid::Uuid::new_v4().to_string();
        let accounts: Vec<_> = (0..2).map(|n| ManagedAccount {
            id: AccountId(uuid::Uuid::new_v4()), alias: format!("fixture-{n}"), email: None,
            framer_user_id: FramerUserId(format!("{prefix}-user-{n}")),
            castle_device_id: format!("fixture-device-{n}"), proxy: None,
            current_team: Some(TeamId(format!("{prefix}-team-{n}"))),
            current_project: Some(ProjectId(format!("fixture-project-{n}"))),
            extra_workspaces: Vec::new(), state: AccountState::Ready, credit: None,
            consecutive_failures: 0, cooldown_step: 0, cooling_until_ms: None,
        }).collect();
        let (tx, calls) = mpsc::unbounded_channel();
        let (loads_tx, loads) = mpsc::unbounded_channel();
        let ports = Arc::new(Ports { records: Mutex::new(accounts.clone()), calls: tx, loads: loads_tx });
        let selected = accounts.iter().map(|a| a.id.0.to_string()).collect::<Vec<_>>().join(",");
        let state = AppState::fixture(
            ports.clone(), ports.clone(), ports.clone(), ports,
            ServingAccountPolicy::parse(Some(&selected)).unwrap(),
            ConcurrencyGate::with_config(GateConfig {
                initial: 1, min: 1, max: 1, recover_ms: 60000, relax_ms: 60000, min_interval_ms: 0,
            }),
        );
        Self { state, accounts, calls, loads }
    }
    // This is the literal model component of the handler's gate key.
    // Messages uses its converted payload model verbatim; the Responses
    // fixture's gpt-6-astra is already its canonical gate model.
    pub fn key(&self, index: usize, gate_model: &str) -> String {
        format!("{}:{}", self.accounts[index].current_team.as_ref().unwrap().0,
            gate_model)
    }
    pub fn headers(&self, index: usize) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-framerize-account", self.accounts[index].id.0.to_string().parse().unwrap());
        headers
    }
    pub async fn next(&mut self) -> Call {
        tokio::time::timeout(Duration::from_secs(3), self.calls.recv()).await
            .expect("handler did not enter expected attempt").expect("fixture closed")
    }
    pub async fn loaded(&mut self, index: usize) {
        let user = self.accounts[index].framer_user_id.clone();
        tokio::time::timeout(Duration::from_secs(3), async {
            while let Some(loaded) = self.loads.recv().await {
                if loaded == user { return; }
            }
            panic!("session fixture closed before expected load");
        }).await.expect("expected account was not loaded");
    }
    pub fn no_call(&mut self) {
        assert!(matches!(self.calls.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "handler bypassed the occupied destination gate");
    }
}

pub fn success(kind: &str) -> Reply {
    match kind {
        "messages" => Reply::Messages(AssembledResponse {
            id: "fixture-msg".into(), content: vec![json!({"type":"text", "text":"fixture"})],
            stop_reason: "end_turn".into(), stop_sequence: None,
            usage: json!({"input_tokens":1,"output_tokens":1}),
            input_tokens: 1, output_tokens: 1, chunks: 1, metered_chunks: 0,
        }),
        "responses" => Reply::Responses(AssembledResponses {
            id: "fixture-response".into(), created_at: 1,
            output: vec![json!({"type":"message","id":"msg_fixture","role":"assistant",
                "status":"completed","content":[{"type":"output_text","text":"fixture","annotations":[]} ]})],
            status: "completed".into(), incomplete_details: None,
            usage: json!({"input_tokens":1,"output_tokens":1}), service_tier: Value::Null,
            input_tokens: 1, output_tokens: 1, chunks: 1, metered_chunks: 0,
        }),
        _ => panic!("raw reply must supply its body"),
    }
}
