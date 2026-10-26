//! Pool integration fixtures. Every port is in memory; no browser or network I/O.
use super::{activate, config, configure, lease_account, management, park, AccountBody, ConfigUpdate};
use super::lease_account_observed;
use crate::{ApiError, AppState, PoolConfig, ServingAccountPolicy, POOL_SETTING_KEY};
use axum::{body::Bytes, extract::State, http::HeaderMap, Json};
use crate::response_output::consume_body as to_bytes;
use framer_core::port::{ResponseResult, ResponsesSink, ValidatedChunk};
use framer_core::account::{AccountState, ManagedAccount};
use framer_core::browser_lifecycle::{BrowserAccountState, BrowserAccountStatus, IdleCloseOutcome, IdleCloseResult};
use framer_core::convert::{AssembledResponse, AssembledResponses, ChainEvent, ClientDialect};
use framer_core::ids::{AccountId, FramerUserId, ProjectId, TeamId};
use framer_core::pipeline::{InferenceRequest, InferenceResult};
use framer_core::port::{BrowserBridge, CastleToken, ChainConfig, FramerClient, ProxyResponse, Session, SessionStore, StateStore, WorkspaceSpec};
use framer_core::resident_pool::ResidentSnapshot;
use framer_core::workspace::CreditSnapshot;
use futures::{future::BoxFuture, StreamExt};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;




#[path = "account_health_tests.rs"]
mod account_health;
#[path = "pool_ownership_tests.rs"]
mod ownership_merge_tests;
#[path = "pool_repick_cleanup_tests.rs"]
mod repick_cleanup;
#[path = "legacy_request_completion_tests.rs"]
mod legacy_request_completion;
#[path = "legacy_pool_completion_tests.rs"]
mod legacy_pool_completion;

// The existing endpoint fixture is private and intentionally denies valid I/O.
// Match its boxed-future convention without adding an async-trait dependency.
macro_rules! denied {
    ($name:ident <$($lt:lifetime),*> ($($arg:ident: $ty:ty),*) -> $output:ty) => {
        fn $name<'s, $($lt,)* 'a>(&'s self, $($arg: $ty),*) -> BoxFuture<'a, $output>
        where 's: 'a, $($lt: 'a,)* Self: 'a {
            Box::pin(async move { panic!("unexpected fixture I/O: {}", stringify!($name)) })
        }
    };
}

#[derive(Clone, Copy, Debug)]
enum CloseMode { Confirm, WrongAccount, WrongHost, WrongIncarnation, StillResident, Busy, BusyResident, Unconfirmed, ActiveCommand, Error }
#[derive(Clone, Copy)]
enum StatusMode { Ready, Busy, WrongAccount, Error, Closing, Unconfirmed, UnconfirmedFor(AccountId) }

struct Ports {
    records: Mutex<Vec<ManagedAccount>>,
    allow_import: AtomicBool,
    saved_sessions: Mutex<Vec<Session>>,
    registered_proxies: Mutex<HashMap<AccountId, Option<String>>>,
    mint_routes: Mutex<Vec<(AccountId, Option<String>)>>,
    settings: Mutex<HashMap<String, String>>,
    pause_list: AtomicBool,
    list_captured: AtomicUsize,
    list_gate: Semaphore,
    save_failure: AtomicBool,
    setting_writes: AtomicUsize,
    account_writes: AtomicUsize,
    pause_usage: AtomicBool, usage_captured: AtomicUsize, usage_gate: Semaphore,
    live: Mutex<HashMap<AccountId, u64>>,
    next_incarnation: AtomicUsize,
    max_live: AtomicUsize,
    events: Mutex<Vec<(&'static str, AccountId)>>,
    close_mode: Mutex<CloseMode>,
    status_mode: Mutex<StatusMode>,
    supports_status: AtomicBool,
    hold_status: AtomicBool,
    status_captured: AtomicUsize,
    status_gate: Semaphore,
    cleanup_on_mint: AtomicBool,
    cleanup_after_content: AtomicBool,
    plain_json: AtomicBool,
    inference_error: Mutex<Option<String>>,
    proxy_calls: Mutex<Vec<(FramerUserId, AccountId, ProjectId, String, String)>>,
    proxy_chunks: Mutex<Option<[Bytes; 2]>>,
    proxy_error: Mutex<Option<String>>,
    mint_control: Mutex<Option<(AccountId, tokio::sync::oneshot::Receiver<Result<(), String>>)>>,
    auth_begin_reply: Mutex<Option<Result<(), String>>>,
    auth_complete_reply: Mutex<Option<Result<(String, Value), String>>>,
    rotate_reply: Mutex<Option<Result<(), String>>>,
    remove_reply: Mutex<Option<Result<(), String>>>,
    session_error: Mutex<Option<String>>,
    gate_close: AtomicBool,
    close_gate: Semaphore,
    request_gate: Semaphore,
    fail_first_account: AtomicBool,
    free_overload_once: AtomicBool,
    request_started: AtomicUsize,
    observed_chats: Mutex<Vec<(AccountId, String)>>,
    request_steps: AtomicUsize,
    request_finished: AtomicUsize,
    request_drops: Arc<AtomicUsize>,
    response_gate: Arc<Semaphore>,
    response_drops: Arc<AtomicUsize>,
}

struct CountDrop(Arc<AtomicUsize>);
impl Drop for CountDrop {
    fn drop(&mut self) { self.0.fetch_add(1, Ordering::SeqCst); }
}

impl Ports {
    fn new(count: u128) -> Self {
        Self {
            records: Mutex::new((1..=count).map(account).collect()),
            allow_import: AtomicBool::new(false), saved_sessions: Mutex::new(Vec::new()),
            registered_proxies: Mutex::new(HashMap::new()), mint_routes: Mutex::new(Vec::new()),
            settings: Mutex::new(HashMap::new()), save_failure: AtomicBool::new(false),
            pause_list: AtomicBool::new(false), list_captured: AtomicUsize::new(0),
            list_gate: Semaphore::new(0),
            setting_writes: AtomicUsize::new(0), account_writes: AtomicUsize::new(0),
            pause_usage: AtomicBool::new(false), usage_captured: AtomicUsize::new(0),
            usage_gate: Semaphore::new(0),
            live: Mutex::new(HashMap::new()), next_incarnation: AtomicUsize::new(1),
            max_live: AtomicUsize::new(0), events: Mutex::new(Vec::new()),
            close_mode: Mutex::new(CloseMode::Confirm), status_mode: Mutex::new(StatusMode::Ready),
            supports_status: AtomicBool::new(true), hold_status: AtomicBool::new(false),
            status_captured: AtomicUsize::new(0), status_gate: Semaphore::new(0),
            cleanup_on_mint: AtomicBool::new(false),
            cleanup_after_content: AtomicBool::new(false),
            plain_json: AtomicBool::new(false),
            inference_error: Mutex::new(None), session_error: Mutex::new(None),
            proxy_calls: Mutex::new(Vec::new()), proxy_chunks: Mutex::new(None),
            proxy_error: Mutex::new(None), mint_control: Mutex::new(None),
            auth_begin_reply: Mutex::new(None), auth_complete_reply: Mutex::new(None),
            rotate_reply: Mutex::new(None), remove_reply: Mutex::new(None),
            gate_close: AtomicBool::new(false), close_gate: Semaphore::new(0),
            request_gate: Semaphore::new(0), fail_first_account: AtomicBool::new(false),
            free_overload_once: AtomicBool::new(false),
            request_started: AtomicUsize::new(0),
            observed_chats: Mutex::new(Vec::new()),
            request_steps: AtomicUsize::new(0), request_finished: AtomicUsize::new(0),
            request_drops: Arc::new(AtomicUsize::new(0)),
            response_gate: Arc::new(Semaphore::new(0)), response_drops: Arc::new(AtomicUsize::new(0)),
        }
    }
    fn event(&self, kind: &'static str, id: AccountId) {
        self.events.lock().unwrap().push((kind, id));
    }
    fn status(&self, id: AccountId) -> BrowserAccountStatus {
        let live = self.live.lock().unwrap();
        BrowserAccountStatus {
            account_id: id, host_generation: Some("fixture-host".into()),
            account_incarnation: live.get(&id).copied().unwrap_or(0),
            state: if live.contains_key(&id) { BrowserAccountState::Resident } else { BrowserAccountState::Absent },
            active_commands: 0,
        }
    }
    async fn step(&self) {
        self.request_gate.acquire().await.unwrap().forget();
        self.request_steps.fetch_add(1, Ordering::SeqCst);
    }
}

impl BrowserBridge for Ports {
    fn supports_browser_lifecycle(&self) -> bool { self.supports_status.load(Ordering::SeqCst) }

    fn castle_token<'s, 'b, 'a>(&'s self, account: &'b AccountId) -> BoxFuture<'a, Result<CastleToken, String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            let control = {
                let mut slot = self.mint_control.lock().unwrap();
                if slot.as_ref().is_some_and(|(target, _)| target == account) {
                    slot.take().map(|(_, control)| control)
                } else { None }
            };
            if let Some(control) = control {
                self.event("mint_wait", *account);
                control.await.expect("fixture mint control dropped")?;
            }
            if *account == id(1) && self.cleanup_on_mint.load(Ordering::SeqCst) {
                self.event("cleanup_unconfirmed", *account);
                return Err("account browser cleanup is unconfirmed; retry after cleanup".into());
            }
            let mut live = self.live.lock().unwrap();
            live.entry(*account).or_insert_with(|| self.next_incarnation.fetch_add(1, Ordering::SeqCst) as u64);
            self.max_live.fetch_max(live.len(), Ordering::SeqCst);
            let proxy = self.registered_proxies.lock().unwrap().get(account).cloned().flatten();
            self.mint_routes.lock().unwrap().push((*account, proxy));
            self.event("mint", *account);
            Ok(CastleToken("fixture-token".into()))
        })
    }
    fn browser_status<'s, 'b, 'a>(&'s self, account: &'b AccountId) -> BoxFuture<'a, Result<BrowserAccountStatus, String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            self.event("status", *account);
            let mut status = self.status(*account);
            match *self.status_mode.lock().unwrap() {
                StatusMode::Ready => {},
                StatusMode::Busy => status.active_commands = 1,
                StatusMode::WrongAccount => status.account_id = id(999),
                StatusMode::Error => return Err("fixture status failure".into()),
                StatusMode::Closing => status.state = BrowserAccountState::Closing,
                StatusMode::Unconfirmed => status.state = BrowserAccountState::Unconfirmed,
                StatusMode::UnconfirmedFor(target) => {
                    if target == *account { status.state = BrowserAccountState::Unconfirmed; }
                }
            }
            if self.hold_status.swap(false, Ordering::SeqCst) {
                self.status_captured.fetch_add(1, Ordering::SeqCst);
                self.status_gate.acquire().await.unwrap().forget();
            }
            Ok(status)
        })
    }
    fn close_idle_browser<'s, 'b, 'c, 'a>(&'s self, account: &'b AccountId, expected: &'c BrowserAccountStatus)
        -> BoxFuture<'a, Result<IdleCloseResult, String>>
    where 's: 'a, 'b: 'a, 'c: 'a, Self: 'a {
        Box::pin(async move {
            self.event("close_started", *account);
            assert!(self.status(*account).same_identity(expected));
            if self.gate_close.load(Ordering::SeqCst) {
                self.close_gate.acquire().await.unwrap().forget();
            }
            let mut status = expected.clone();
            status.state = BrowserAccountState::Absent;
            let mut outcome = IdleCloseOutcome::Closed;
            match *self.close_mode.lock().unwrap() {
                CloseMode::Confirm => {
                    self.live.lock().unwrap().remove(account);
                    self.event("close_confirmed", *account);
                },
                CloseMode::WrongAccount => status.account_id = id(999),
                CloseMode::WrongHost => status.host_generation = Some("other-host".into()),
                CloseMode::WrongIncarnation => status.account_incarnation += 1,
                CloseMode::StillResident => status.state = BrowserAccountState::Resident,
                CloseMode::Busy => outcome = IdleCloseOutcome::Busy,
                CloseMode::BusyResident => {
                    outcome = IdleCloseOutcome::Busy;
                    status.state = BrowserAccountState::Resident;
                    status.active_commands = 1;
                },
                CloseMode::Unconfirmed => outcome = IdleCloseOutcome::Unconfirmed,
                CloseMode::ActiveCommand => status.active_commands = 1,
                CloseMode::Error => return Err("fixture close failure".into()),
            }
            Ok(IdleCloseResult { outcome, status })
        })
    }
    fn register_account_proxy<'s, 'b, 'a>(&'s self, account: &'b AccountId, proxy: Option<String>)
        -> BoxFuture<'a, Result<(), String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            let live = self.live.lock().unwrap();
            let mut registered = self.registered_proxies.lock().unwrap();
            if live.contains_key(account) && registered.get(account) != Some(&proxy) {
                return Err("fixture proxy change requires confirmed absence".into());
            }
            registered.insert(*account, proxy);
            self.event("register_proxy", *account);
            Ok(())
        })
    }
    fn auth_begin<'s, 'b, 'c, 'a>(&'s self, account: &'b AccountId, _email: &'c str)
        -> BoxFuture<'a, Result<(), String>>
    where 's: 'a, 'b: 'a, 'c: 'a, Self: 'a {
        Box::pin(async move {
            self.event("auth_begin", *account);
            self.auth_begin_reply.lock().unwrap().take()
                .expect("unexpected fixture I/O: auth_begin")
        })
    }
    fn auth_complete<'s, 'b, 'c, 'a>(&'s self, account: &'b AccountId, _token: &'c str)
        -> BoxFuture<'a, Result<(String, Value), String>>
    where 's: 'a, 'b: 'a, 'c: 'a, Self: 'a {
        Box::pin(async move {
            self.event("auth_complete", *account);
            self.auth_complete_reply.lock().unwrap().take()
                .expect("unexpected fixture I/O: auth_complete")
        })
    }
    fn rotate_identity<'s, 'b, 'a>(&'s self, account: &'b AccountId)
        -> BoxFuture<'a, Result<(), String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            self.event("rotate_identity", *account);
            self.rotate_reply.lock().unwrap().take()
                .unwrap_or(Ok(()))
        })
    }
}

impl StateStore for Ports {
    fn get<'s, 'a>(&'s self, id: AccountId) -> BoxFuture<'a, Result<Option<ManagedAccount>, String>>
    where 's: 'a, Self: 'a {
        Box::pin(async move { Ok(self.records.lock().unwrap().iter().find(|a| a.id == id).cloned()) })
    }
    fn list<'s, 'a>(&'s self) -> BoxFuture<'a, Result<Vec<ManagedAccount>, String>>
    where 's: 'a, Self: 'a {
        Box::pin(async move {
            let records = self.records.lock().unwrap().clone();
            if self.pause_list.swap(false, Ordering::SeqCst) {
                self.list_captured.fetch_add(1, Ordering::SeqCst);
                self.list_gate.acquire().await.unwrap().forget();
            }
            Ok(records)
        })
    }
    fn upsert<'s, 'b, 'a>(&'s self, account: &'b ManagedAccount) -> BoxFuture<'a, Result<(), String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            self.account_writes.fetch_add(1, Ordering::SeqCst);
            *self.records.lock().unwrap().iter_mut().find(|a| a.id == account.id).unwrap() = account.clone();
            Ok(())
        })
    }
    fn save_setting<'s, 'b, 'c, 'a>(&'s self, key: &'b str, value: &'c str) -> BoxFuture<'a, Result<(), String>>
    where 's: 'a, 'b: 'a, 'c: 'a, Self: 'a {
        Box::pin(async move {
            self.setting_writes.fetch_add(1, Ordering::SeqCst);
            if self.save_failure.load(Ordering::SeqCst) { return Err("fixture settings failure".into()); }
            self.settings.lock().unwrap().insert(key.into(), value.into());
            Ok(())
        })
    }
    fn remove<'s, 'a>(&'s self, id: AccountId) -> BoxFuture<'a, Result<(), String>>
    where 's: 'a, Self: 'a {
        Box::pin(async move {
            self.event("remove", id);
            self.account_writes.fetch_add(1, Ordering::SeqCst);
            self.remove_reply.lock().unwrap().take()
                .expect("unexpected fixture I/O: remove")?;
            self.records.lock().unwrap().retain(|account| account.id != id);
            Ok(())
        })
    }
}

impl SessionStore for Ports {
    fn load<'s, 'b, 'a>(&'s self, user: &'b FramerUserId) -> BoxFuture<'a, Result<Option<Session>, String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            if let Some(error) = self.session_error.lock().unwrap().clone() { return Err(error); }
            Ok(Some(Session { active_user: user.clone(), cookie_header: "session=fixture-only".into() }))
        })
    }
    fn save<'s, 'b, 'a>(&'s self, session: &'b Session) -> BoxFuture<'a, Result<(), String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            assert!(self.allow_import.load(Ordering::SeqCst), "unexpected fixture I/O: save");
            assert_eq!(session.active_user.0, "user-2");
            assert_eq!(session.cookie_header, "session=fixture-only");
            self.saved_sessions.lock().unwrap().push(session.clone());
            Ok(())
        })
    }
    denied!(delete<'b>(_user: &'b FramerUserId) -> Result<(), String>);
}

impl FramerClient for Ports {
    denied!(request_magic_link<'b, 'c>(_email: &'b str, _castle: &'c CastleToken) -> Result<(), String>);
    denied!(complete_magic_link<'b, 'c>(_token: &'b str, _castle: &'c CastleToken) -> Result<Session, String>);
    fn me<'s, 'b, 'a>(&'s self, session: &'b Session) -> BoxFuture<'a, Result<Value, String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            assert!(self.allow_import.load(Ordering::SeqCst), "unexpected fixture I/O: me");
            assert_eq!(session.cookie_header, "session=fixture-only");
            assert!(session.active_user.0.is_empty() || session.active_user.0 == "user-2");
            Ok(json!({"id":"user-2","spaces":[{"scope":"organization","ownerId":"team-2"}]}))
        })
    }
    fn run_inference<'s, 'b, 'c, 'd, 'e, 'f, 'a>(
        &'s self, session: &'b Session, _browser: &'c dyn BrowserBridge,
        account: &'d AccountId, _project: &'e ProjectId, _req: &'f InferenceRequest,
    ) -> BoxFuture<'a, Result<InferenceResult, String>>
    where 's: 'a, 'b: 'a, 'c: 'a, 'd: 'a, 'e: 'a, 'f: 'a, Self: 'a {
        Box::pin(async move {
            assert_eq!(session.active_user.0, format!("user-{}", account.0.as_u128()));
            let error = self.inference_error.lock().unwrap().clone()
                .expect("unexpected fixture I/O: run_inference");
            self.request_started.fetch_add(1, Ordering::SeqCst);
            self.event("run_inference", *account);
            Err(error)
        })
    }
    denied!(create_workspace<'b, 'c>(_session: &'b Session, _spec: &'c WorkspaceSpec) -> Result<TeamId, String>);
    fn freechat_long<'s, 'b, 'c, 'd, 'e, 'f, 'g, 'h, 'a>(
        &'s self, session: &'b Session, browser: &'c dyn BrowserBridge, account: &'d AccountId,
        _project: &'e ProjectId, _payload: Value, _cfg: &'f ChainConfig, chat: &'g str,
        events: Option<&'h (dyn Fn(ChainEvent) + Send + Sync)>,
    ) -> BoxFuture<'a, Result<AssembledResponse, String>>
    where 's: 'a, 'b: 'a, 'c: 'a, 'd: 'a, 'e: 'a, 'f: 'a, 'g: 'a, 'h: 'a, Self: 'a {
        Box::pin(async move {
            let _drop = CountDrop(self.request_drops.clone());
            self.observed_chats.lock().unwrap().push((*account, chat.to_string()));
            self.request_started.fetch_add(1, Ordering::SeqCst);
            assert_eq!(session.active_user.0, format!("user-{}", account.0.as_u128()));
            browser.castle_token(account).await?;
            if self.fail_first_account.load(Ordering::SeqCst) && *account == id(1) {
                return Err(self.inference_error.lock().unwrap().clone()
                    .unwrap_or_else(|| "fixture risk-refused".into()));
            }
            if self.free_overload_once.swap(false, Ordering::SeqCst) {
                return Err("retryable 429 fixture".into());
            }
            for _ in 0..2 {
                self.step().await;
                if let Some(callback) = events {
                    callback(ChainEvent::Chunk(vec![json!({"type":"text","text":"fixture delta"})]));
                }
                if self.cleanup_after_content.load(Ordering::SeqCst) && *account == id(1) {
                    return Err("chunk 0 retry died (fallback off): account browser cleanup is unconfirmed; retry after cleanup".into());
                }
            }
            self.request_finished.fetch_add(1, Ordering::SeqCst);
            Ok(AssembledResponse {
                id: "fixture-message".into(), content: vec![json!({"type":"text","text":"fixture result"})],
                stop_reason: "end_turn".into(), stop_sequence: None,
                usage: json!({"input_tokens":1,"output_tokens":2}),
                input_tokens: 1, output_tokens: 2, chunks: 2, metered_chunks: 0,
            })
        })
    }
    fn native_long_with_sink<'s, 'b, 'c, 'd, 'e, 'f, 'g, 'h, 'a>(
        &'s self, session: &'b Session, browser: &'c dyn BrowserBridge, account: &'d AccountId,
        _project: &'e ProjectId, _payload: Value, _cfg: &'f ChainConfig, chat: &'g str,
        sink: &'h dyn ResponsesSink,
    ) -> BoxFuture<'a, ResponseResult>
    where 's: 'a, 'b: 'a, 'c: 'a, 'd: 'a, 'e: 'a, 'f: 'a, 'g: 'a, 'h: 'a, Self: 'a {
        Box::pin(async move {
            let _drop = CountDrop(self.request_drops.clone());
            self.observed_chats.lock().unwrap().push((*account, chat.to_string()));
            self.request_started.fetch_add(1, Ordering::SeqCst);
            assert_eq!(session.active_user.0, format!("user-{}", account.0.as_u128()));
            browser.castle_token(account).await?;
            if self.fail_first_account.load(Ordering::SeqCst) && *account == id(1) {
                return Err(self.inference_error.lock().unwrap().clone()
                    .unwrap_or_else(|| "fixture risk-refused".into()).into());
            }
            let output = vec![json!({"id":"fixture-item","type":"message","role":"assistant",
                "status":"completed","content":[{"type":"output_text","text":"fixture result","annotations":[]}]})];
            let value = AssembledResponses {
                id: "fixture-response".into(), created_at: 1, output, status: "completed".into(),
                incomplete_details: None, usage: json!({"input_tokens":1,"output_tokens":2,"total_tokens":3}),
                service_tier: Value::Null, input_tokens: 1, output_tokens: 2, chunks: 2, metered_chunks: 0,
            };
            for n in 0..2 {
                self.step().await;
                let previous = if n == 0 { &[][..] } else { value.output.as_slice() };
                let chunk = ValidatedChunk::validate(value.output.clone(), previous, n > 0)?;
                sink.emit(chunk).await?;
                if self.cleanup_after_content.load(Ordering::SeqCst) && *account == id(1) {
                    return Err("chunk 0 retry died (fallback off): account browser cleanup is unconfirmed; retry after cleanup".into());
                }
            }
            self.request_finished.fetch_add(1, Ordering::SeqCst);
            Ok(value)
        })
    }
    fn native_long<'s, 'b, 'c, 'd, 'e, 'f, 'g, 'h, 'a>(
        &'s self, session: &'b Session, browser: &'c dyn BrowserBridge, account: &'d AccountId,
        _project: &'e ProjectId, _payload: Value, _cfg: &'f ChainConfig, chat: &'g str,
        events: Option<&'h (dyn Fn(ChainEvent) + Send + Sync)>,
    ) -> BoxFuture<'a, Result<AssembledResponses, String>>
    where 's: 'a, 'b: 'a, 'c: 'a, 'd: 'a, 'e: 'a, 'f: 'a, 'g: 'a, 'h: 'a, Self: 'a {
        Box::pin(async move {
            let _drop = CountDrop(self.request_drops.clone());
            self.observed_chats.lock().unwrap().push((*account, chat.to_string()));
            self.request_started.fetch_add(1, Ordering::SeqCst);
            assert_eq!(session.active_user.0, format!("user-{}", account.0.as_u128()));
            browser.castle_token(account).await?;
            if self.fail_first_account.load(Ordering::SeqCst) && *account == id(1) {
                return Err(self.inference_error.lock().unwrap().clone()
                    .unwrap_or_else(|| "fixture risk-refused".into()));
            }
            let output = vec![json!({"id":"fixture-item","type":"message","role":"assistant",
                "status":"completed","content":[{"type":"output_text","text":"fixture result","annotations":[]}]})];
            for n in 0..2 {
                self.step().await;
                if let Some(callback) = events {
                    callback(ChainEvent::ResponsesItems { items: output.clone(), continues_text: n > 0 });
                }
                if self.cleanup_after_content.load(Ordering::SeqCst) && *account == id(1) {
                    return Err("chunk 0 retry died (fallback off): account browser cleanup is unconfirmed; retry after cleanup".into());
                }
            }
            self.request_finished.fetch_add(1, Ordering::SeqCst);
            Ok(AssembledResponses {
                id: "fixture-response".into(), created_at: 1, output, status: "completed".into(),
                incomplete_details: None, usage: json!({"input_tokens":1,"output_tokens":2,"total_tokens":3}),
                service_tier: Value::Null, input_tokens: 1, output_tokens: 2, chunks: 2, metered_chunks: 0,
            })
        })
    }
    fn proxy<'s, 'b, 'c, 'd, 'e, 'f, 'g, 'a>(
        &'s self, session: &'b Session, browser: &'c dyn BrowserBridge, account: &'d AccountId,
        project: &'e ProjectId, dialect: &'f str, _payload: Value, chat: &'g str,
    ) -> BoxFuture<'a, Result<ProxyResponse, String>>
    where 's: 'a, 'b: 'a, 'c: 'a, 'd: 'a, 'e: 'a, 'f: 'a, 'g: 'a, Self: 'a {
        Box::pin(async move {
            let _drop = CountDrop(self.request_drops.clone());
            self.proxy_calls.lock().unwrap().push((session.active_user.clone(), *account,
                project.clone(), dialect.into(), chat.into()));
            let chunks = self.proxy_chunks.lock().unwrap().take();
            self.request_started.fetch_add(1, Ordering::SeqCst);
            assert_eq!(session.active_user.0, format!("user-{}", account.0.as_u128()));
            browser.castle_token(account).await?;
            self.step().await;
            self.step().await;
            self.request_finished.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = self.proxy_error.lock().unwrap().take() { return Err(error); }
            let plain_json = self.plain_json.load(Ordering::SeqCst);
            let state = (self.response_gate.clone(), CountDrop(self.response_drops.clone()), 0,
                self.cleanup_after_content.load(Ordering::SeqCst), chunks);
            let stream = futures::stream::unfold(state, move |(gate, held, n, cleanup, chunks)| async move {
                if n == 2 { return None; }
                gate.acquire().await.unwrap().forget();
                if n == 1 && cleanup {
                    return Some((Err("account browser cleanup is unconfirmed; retry after cleanup".into()),
                        (gate, held, n + 1, cleanup, chunks)));
                }
                let bytes = if plain_json {
                    if n == 0 { b"{\"fixture\":".as_slice() } else { b"true}".as_slice() }
                } else if n == 0 { b"data: {\"fixture\":true}\n\n".as_slice() }
                    else { b"data: [DONE]\n\n".as_slice() };
                let bytes = chunks.as_ref().map_or_else(|| Bytes::from_static(bytes),
                    |chunks| chunks[n].clone());
                Some((Ok::<Bytes, String>(bytes), (gate, held, n + 1, cleanup, chunks)))
            });
            let content_type = if plain_json { "application/json" } else { "text/event-stream" };
            Ok(ProxyResponse { status: 200, content_type: content_type.into(),
                stream: Box::pin(stream), queue_wait_ms: 0 })
        })
    }
    fn usage<'s, 'b, 'c, 'a>(&'s self, _session: &'b Session, _team: &'c TeamId) -> BoxFuture<'a, Result<CreditSnapshot, String>>
    where 's: 'a, 'b: 'a, 'c: 'a, Self: 'a {
        Box::pin(async move {
            if self.pause_usage.swap(false, Ordering::SeqCst) {
                self.usage_captured.fetch_add(1, Ordering::SeqCst);
                self.usage_gate.acquire().await.unwrap().forget();
            }
            Ok(CreditSnapshot { available: 1000.0, funds: 1, grants: 1, allocated: 1000.0 })
        })
    }
}

fn id(n: u128) -> AccountId { AccountId(uuid::Uuid::from_u128(n)) }
fn account(n: u128) -> ManagedAccount {
    ManagedAccount {
        id: id(n), alias: format!("fixture-{n}"), email: Some(format!("fixture-{n}@example.invalid")),
        framer_user_id: FramerUserId(format!("user-{n}")), castle_device_id: format!("device-{n}"),
        proxy: Some(format!("http://proxy-{n}.invalid:8080")),
        current_team: Some(TeamId(format!("team-{n}"))), current_project: Some(ProjectId(format!("project-{n}"))),
        extra_workspaces: vec![], state: AccountState::Ready, credit: None,
        // Distinct ranks make the initial automatic pick deterministic without
        // disabling retry by pinning an account header. Ties use the runtime cursor.
        consecutive_failures: (n - 1) as u32, cooldown_step: 2, cooling_until_ms: None,
    }
}
fn fixture(capacity: usize, saved: u128) -> (AppState, Arc<Ports>) {
    let ports = Arc::new(Ports::new(saved));
    let state = AppState::fixture(
        ports.clone(), ports.clone(), ports.clone(), ports.clone(),
        ServingAccountPolicy::from_config(PoolConfig {
            active_capacity: capacity, ..Default::default()
        }).unwrap(),
        framer_core::gate::ConcurrencyGate::with_config(
            framer_core::gate::GateConfig { initial: 5, min: 4, max: 8,
                recover_ms: 1_000, relax_ms: 30_000, min_interval_ms: 0 }),
    );
    (state, ports)
}
fn update(value: Value) -> ConfigUpdate { serde_json::from_value(value).unwrap() }
fn slot(state: &AppState, account: AccountId) -> ResidentSnapshot {
    state.serving_accounts.residents.snapshot().unwrap().residents.into_iter()
        .find(|slot| slot.account == account).expect("account must retain its slot")
}
fn records(ports: &Ports) -> Value { serde_json::to_value(&*ports.records.lock().unwrap()).unwrap() }
fn stored_config(ports: &Ports) -> PoolConfig {
    serde_json::from_str(ports.settings.lock().unwrap().get(POOL_SETTING_KEY).unwrap()).unwrap()
}

async fn activated(state: &AppState, n: u128) {
    activate(State(state.clone()), Json(AccountBody { account_id: id(n) })).await.unwrap();
}

#[tokio::test]
async fn negative_browser_state_is_fenced_and_reported_unavailable() {
    for mode in [StatusMode::Closing, StatusMode::Unconfirmed] {
        let (state, ports) = fixture(2, 3);
        activated(&state, 1).await;
        let before = records(&ports);
        *ports.status_mode.lock().unwrap() = mode;
        let outcome = lease_account_observed(&state, id(1)).await;
        assert!(matches!(outcome.result, Err(ApiError::PoolBusy(_))));
        assert!(outcome.cleanup_confirmed, "a completed status read owns no unfinished work");
        assert!(slot(&state, id(1)).uncertain);
        assert_eq!(slot(&state, id(1)).users, 0);
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 1);
        let Json(view) = crate::pool_shape(State(state.clone())).await.unwrap();
        assert_eq!(view["available_now"], 2);
        let entries: Vec<_> = view["workspaces"].as_array().unwrap().iter()
            .flat_map(|team| team["accounts"].as_array().unwrap()).collect();
        let bad = entries.iter().find(|a| a["account_id"] == json!(id(1))).unwrap();
        assert_eq!(bad["available"], false);
        assert_eq!(records(&ports), before);
        *ports.status_mode.lock().unwrap() = StatusMode::Ready;
        let pool = ports.records.lock().unwrap().clone();
        assert_ne!(state.serving_accounts.select(pool.clone(), None).unwrap().id, id(1));
        assert!(matches!(state.serving_accounts.select(pool, Some(&id(1).0.to_string())),
            Err(ApiError::PoolBusy(_))));
    }
}
async fn parked(state: &AppState, n: u128) -> Result<Json<Value>, ApiError> {
    park(State(state.clone()), Json(AccountBody { account_id: id(n) })).await
}

#[tokio::test]
async fn uncertain_observation_does_not_drop_an_existing_request_lease() {
    let (state, ports) = fixture(1, 2);
    activated(&state, 1).await;
    let held = lease_account(&state, id(1)).await.unwrap();
    *ports.status_mode.lock().unwrap() = StatusMode::Unconfirmed;
    assert!(lease_account(&state, id(1)).await.is_err());
    assert_eq!(slot(&state, id(1)).users, 1);
    assert!(slot(&state, id(1)).uncertain);
    assert!(matches!(crate::current_account(&state, id(1)).await, Err(ApiError::PoolBusy(_))));
    assert!(parked(&state, 1).await.is_err());
    assert!(!ports.events.lock().unwrap().iter().any(|event| event.0 == "close_started"));
    drop(held);
    assert_eq!(slot(&state, id(1)).users, 0);
    assert!(slot(&state, id(1)).uncertain);
}

#[tokio::test]
async fn failed_or_mismatched_status_cannot_admit_a_resident() {
    for mode in [StatusMode::WrongAccount, StatusMode::Error] {
        let (state, ports) = fixture(1, 2);
        activated(&state, 1).await;
        *ports.status_mode.lock().unwrap() = mode;
        let outcome = lease_account_observed(&state, id(1)).await;
        assert!(outcome.result.is_err());
        assert!(!outcome.cleanup_confirmed, "an unknown status cannot confirm cleanup");
        assert!(slot(&state, id(1)).uncertain);
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 1);
    }
}
#[tokio::test]
async fn delayed_healthy_status_cannot_bypass_another_requests_uncertainty() {
    let (state, ports) = fixture(1, 2);
    activated(&state, 1).await;
    ports.hold_status.store(true, Ordering::SeqCst);
    let mut first = Box::pin(lease_account_observed(&state, id(1)));
    assert!(futures::poll!(first.as_mut()).is_pending());
    assert_eq!(ports.status_captured.load(Ordering::SeqCst), 1);
    *ports.status_mode.lock().unwrap() = StatusMode::Unconfirmed;
    assert!(lease_account(&state, id(1)).await.is_err());
    assert_eq!(slot(&state, id(1)).users, 1);
    ports.status_gate.add_permits(1);
    let outcome = first.await;
    assert!(matches!(outcome.result, Err(ApiError::PoolBusy(_))));
    assert!(outcome.cleanup_confirmed, "local admission refusal started no external work");
    assert!(slot(&state, id(1)).uncertain);
    assert_eq!(slot(&state, id(1)).users, 0);
}

#[tokio::test]
async fn unsupported_lifecycle_bridge_preserves_admission_without_inspection() {
    let (state, ports) = fixture(1, 1);
    ports.supports_status.store(false, Ordering::SeqCst);
    *ports.status_mode.lock().unwrap() = StatusMode::Error;
    activated(&state, 1).await;
    assert_eq!(*ports.events.lock().unwrap(), vec![("mint", id(1))]);
    assert!(!slot(&state, id(1)).uncertain);
}

#[tokio::test]
async fn status_timeout_keeps_occupied_slot_without_closing_the_browser() {
    let (state, ports) = fixture(1, 2);
    activated(&state, 1).await;
    let before = records(&ports);
    ports.hold_status.store(true, Ordering::SeqCst);
    let outcome = lease_account_observed(&state, id(1)).await;
    assert!(matches!(outcome.result, Err(ApiError::PoolBusy(_))));
    assert!(!outcome.cleanup_confirmed, "a timed-out read remains unconfirmed");
    assert_eq!(ports.status_captured.load(Ordering::SeqCst), 1);
    assert!(slot(&state, id(1)).uncertain);
    assert_eq!(slot(&state, id(1)).users, 0);
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 1);
    assert_eq!(ports.live.lock().unwrap().len(), 1);
    assert!(!ports.events.lock().unwrap().iter().any(|event| event.0 == "close_started"));
    assert_eq!(records(&ports), before);
}

#[tokio::test]
async fn all_uncertain_slots_report_zero_availability_including_unloaded_accounts() {
    let (state, ports) = fixture(6, 11);
    for n in 1..=6 { activated(&state, n).await; }
    let before = records(&ports);
    *ports.status_mode.lock().unwrap() = StatusMode::Unconfirmed;
    for n in 1..=6 { assert!(lease_account(&state, id(n)).await.is_err()); }
    let Json(view) = crate::pool_shape(State(state.clone())).await.unwrap();
    assert_eq!(view["available_now"], 0);
    let pool = ports.records.lock().unwrap().clone();
    assert!(matches!(state.serving_accounts.select(pool.clone(), None), Err(ApiError::PoolBusy(_))));
    assert!(matches!(state.serving_accounts.select(pool, Some(&id(7).0.to_string())),
        Err(ApiError::PoolBusy(_))));
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 6);
    assert_eq!(ports.live.lock().unwrap().len(), 6);
    assert_eq!(records(&ports), before);
    assert!(!ports.events.lock().unwrap().iter().any(|event| event.0 == "close_started"));
}

#[tokio::test]
async fn nonresident_availability_requires_free_or_unmanaged_idle_capacity() {
    let (state, ports) = fixture(1, 2);
    activated(&state, 1).await;
    let held = lease_account(&state, id(1)).await.unwrap();
    let Json(active) = crate::pool_shape(State(state.clone())).await.unwrap();
    assert_eq!(active["available_now"], 1);
    drop(held);
    let guard = management(&state, id(1), false).await.unwrap();
    let Json(managed) = crate::pool_shape(State(state.clone())).await.unwrap();
    assert_eq!(managed["available_now"], 0);
    drop(guard);
    let Json(idle) = crate::pool_shape(State(state.clone())).await.unwrap();
    assert_eq!(idle["available_now"], 2);
    assert!(!ports.events.lock().unwrap().iter().any(|event| event.0 == "close_started"));
}

#[test]
fn only_exact_local_cleanup_errors_trigger_fencing_and_retry() {
    let local = "account browser cleanup is unconfirmed; retry after cleanup";
    for error in [local.to_owned(), format!("chunk 0 retry died (fallback off): {local}"),
        "browser launch cleanup is unconfirmed".to_owned()] {
        assert!(super::cleanup_unconfirmed(&error));
        assert!(crate::should_retry_account(&error));
    }
    for error in [format!("upstream 403: {local}"), format!("retryable 500: {local}"),
        format!("chunk x retry died (fallback off): {local}"), format!("{local} extra"),
        "account is busy or browser cleanup is unconfirmed".to_owned()] {
        assert!(!super::cleanup_unconfirmed(&error));
        assert!(!crate::should_retry_account(&error));
    }
}

#[tokio::test]
async fn activation_failure_after_healthy_status_fences_without_health_mutation() {
    let (state, ports) = fixture(2, 2);
    let before = records(&ports);
    ports.cleanup_on_mint.store(true, Ordering::SeqCst);
    assert!(activate(State(state.clone()), Json(AccountBody { account_id: id(1) })).await.is_err());
    assert!(slot(&state, id(1)).uncertain);
    assert_eq!(slot(&state, id(1)).users, 0);
    assert!(lease_account(&state, id(1)).await.is_err());
    activated(&state, 2).await;
    assert_eq!(records(&ports), before);
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
    assert!(!ports.events.lock().unwrap().iter().any(|event| event.0 == "close_started"));
}

#[tokio::test]
async fn cleanup_failure_repicks_a_healthy_account_before_any_client_content() {
    for route in [Route::Free, Route::Native] {
        for stream in [false, true] {
            let (state, ports) = fixture(2, 2);
            activated(&state, 1).await;
            activated(&state, 2).await;
            let mut expected = ports.records.lock().unwrap().clone();
            expected[1].note_success();
            ports.cleanup_on_mint.store(true, Ordering::SeqCst);
            ports.request_gate.add_permits(2);
            let response = handler_response(&state, route, stream).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::OK);
            let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            assert!(String::from_utf8_lossy(&body).contains("fixture"));
            assert!(!String::from_utf8_lossy(&body).contains("cleanup is unconfirmed"));
            assert_eq!(ports.request_started.load(Ordering::SeqCst), 2);
            assert_eq!(ports.request_finished.load(Ordering::SeqCst), 1);
            assert!(slot(&state, id(1)).uncertain);
            assert_eq!(slot(&state, id(1)).users, 0);
            assert!(!slot(&state, id(2)).uncertain);
            assert_eq!(records(&ports), serde_json::to_value(expected).unwrap());
            assert_eq!(ports.account_writes.load(Ordering::SeqCst), 1);
            assert!(!ports.events.lock().unwrap().iter().any(|event| event.0 == "close_started"));
        }
    }
}

#[tokio::test]
async fn cleanup_after_streamed_content_fences_without_replaying_on_another_account() {
    for route in [Route::Free, Route::Native] {
        let (state, ports) = fixture(2, 2);
        activated(&state, 1).await;
        activated(&state, 2).await;
        let before = records(&ports);
        ports.cleanup_after_content.store(true, Ordering::SeqCst);
        ports.request_gate.add_permits(1);
        let response = handler_response(&state, route, true).await.unwrap();
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("cleanup is unconfirmed"));
        drop(body);
        // Preserve the cleanup fence while the producer joins output disposal.
        tokio::time::timeout(Duration::from_secs(3), async {
            while slot(&state, id(1)).users != 0 { tokio::task::yield_now().await; }
        }).await.expect("request residency did not settle after body disposal");
        assert_eq!(ports.request_started.load(Ordering::SeqCst), 1);
        assert_eq!(ports.request_finished.load(Ordering::SeqCst), 0);
        assert!(slot(&state, id(1)).uncertain);
        assert_eq!(slot(&state, id(1)).users, 0);
        assert!(!slot(&state, id(2)).uncertain);
        assert_eq!(records(&ports), before);
    }
}

#[tokio::test]
async fn proxy_body_cleanup_failure_fences_the_retained_stream_lease() {
    let (state, ports) = fixture(1, 1);
    ports.cleanup_after_content.store(true, Ordering::SeqCst);
    ports.request_gate.add_permits(2);
    ports.response_gate.add_permits(2);
    let response = handler_response(&state, Route::Compat, true).await.unwrap();
    let error = to_bytes(response.into_body(), 1024 * 1024).await.unwrap_err();
    assert!(error.to_string().contains("cleanup is unconfirmed"));
    assert!(slot(&state, id(1)).uncertain);
    assert_eq!(slot(&state, id(1)).users, 0);
    assert_eq!(ports.request_started.load(Ordering::SeqCst), 1);
    assert_eq!(ports.response_drops.load(Ordering::SeqCst), 1);
    assert_eq!(ports.records.lock().unwrap()[0].consecutive_failures, 0);
    assert_eq!(ports.records.lock().unwrap()[0].cooling_until_ms, None);
}

#[tokio::test]
async fn plain_json_body_cleanup_fences_before_returning_stream_error() {
    let (state, ports) = fixture(2, 2);
    activated(&state, 1).await;
    activated(&state, 2).await;
    ports.plain_json.store(true, Ordering::SeqCst);
    ports.cleanup_after_content.store(true, Ordering::SeqCst);
    ports.request_gate.add_permits(2);
    ports.response_gate.add_permits(2);
    let response = handler_response(&state, Route::Compat, false).await.unwrap();
    let before = records(&ports);
    let writes_before = ports.account_writes.load(Ordering::SeqCst);
    assert_eq!(response.headers()["content-type"], "application/json");
    let mut body = response.into_body().into_data_stream();
    assert!(slot(&state, id(1)).users > 0);
    assert_eq!(body.next().await.unwrap().unwrap().as_ref(), b"{\"fixture\":");
    assert!(!slot(&state, id(1)).uncertain);
    assert!(slot(&state, id(1)).users > 0);
    assert!(body.next().await.unwrap().is_err());
    assert!(slot(&state, id(1)).uncertain);
    assert!(!slot(&state, id(2)).uncertain);
    drop(body);
    assert_eq!(ports.request_started.load(Ordering::SeqCst), 1);
    assert_eq!(ports.response_drops.load(Ordering::SeqCst), 1);
    assert!(!ports.events.lock().unwrap().iter().any(|event| event.0 == "close_started"));
    assert_eq!(slot(&state, id(1)).users, 0);
    assert_eq!(records(&ports), before);
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), writes_before);
}

#[tokio::test]
async fn typed_pipeline_cleanup_fences_only_the_held_account() {
    let (state, ports) = fixture(2, 2);
    activated(&state, 1).await;
    activated(&state, 2).await;
    ports.records.lock().unwrap()[0].credit = Some(CreditSnapshot {
        available: 1000.0, funds: 1, grants: 1, allocated: 1000.0,
    });
    let before = records(&ports);
    *ports.inference_error.lock().unwrap() =
        Some("account browser cleanup is unconfirmed; retry after cleanup".into());
    let outcome = crate::run(State(state.clone()), Json(crate::RunBody {
        account_id: id(1), prompt: "fixture".into(), model: None, chat_session: None,
    })).await;
    let response = outcome.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(body.as_ref(), b"framer: account browser cleanup is unconfirmed; retry after cleanup");
    assert!(slot(&state, id(1)).uncertain);
    assert!(!slot(&state, id(2)).uncertain);
    assert_eq!(ports.request_started.load(Ordering::SeqCst), 1);
    assert!(ports.events.lock().unwrap().contains(&("run_inference", id(1))));
    assert!(!ports.events.lock().unwrap().iter().any(|event| event.0 == "close_started"));
    idle_without_writes(&state, &ports, &before);
}

#[tokio::test]
async fn typed_pipeline_store_and_unrelated_errors_do_not_fence() {
    let local = "account browser cleanup is unconfirmed; retry after cleanup";
    for (name, session_error, inference_error, expected, requests_started) in [
        ("upstream failure", None, Some(format!("upstream 500: {local}")),
            format!("framer: upstream 500: {local}"), 1),
        ("session store failure", Some(local.to_owned()), None,
            format!("store: {local}"), 0),
    ] {
        let (state, ports) = fixture(1, 1);
        activated(&state, 1).await;
        ports.records.lock().unwrap()[0].credit = Some(CreditSnapshot {
            available: 1000.0, funds: 1, grants: 1, allocated: 1000.0,
        });
        let before = records(&ports);
        *ports.session_error.lock().unwrap() = session_error;
        *ports.inference_error.lock().unwrap() = inference_error;
        let outcome = crate::run(State(state.clone()), Json(crate::RunBody {
            account_id: id(1), prompt: "fixture".into(), model: None, chat_session: None,
        })).await;
        let response = outcome.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR, "{name}");
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(body.as_ref(), expected.as_bytes(), "{name}");
        assert!(!slot(&state, id(1)).uncertain, "{name}");
        assert_eq!(ports.request_started.load(Ordering::SeqCst), requests_started, "{name}");
        assert!(!ports.events.lock().unwrap().iter().any(|event| event.0 == "close_started"));
        idle_without_writes(&state, &ports, &before);
    }
}

#[tokio::test]
async fn cleanup_result_cannot_fence_a_different_requests_account() {
    let (state, _) = fixture(2, 2);
    let first = lease_account(&state, id(1)).await.unwrap();
    let second = super::RequestResidency::new(lease_account(&state, id(2)).await.unwrap());
    let result: Result<(), String> = second.observe_result(id(1),
        Err("account browser cleanup is unconfirmed; retry after cleanup".into()));
    assert!(result.unwrap_err().contains("no matching account lease"));
    assert!(!slot(&state, id(1)).uncertain);
    assert!(!slot(&state, id(2)).uncertain);
    assert_eq!(slot(&state, id(1)).users, 1);
    assert_eq!(slot(&state, id(2)).users, 1);
    drop((first, second));
}

async fn changed(state: &AppState, value: Value) -> Result<Json<PoolConfig>, ApiError> {
    configure(State(state.clone()), Json(update(value))).await
}
async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !predicate() { tokio::task::yield_now().await; }
    }).await.expect("fixture did not reach the expected state");
}
fn idle_without_writes(state: &AppState, ports: &Ports, before: &Value) {
    assert!(state.serving_accounts.residents.snapshot().unwrap().residents.iter().all(|s| s.users == 0));
    assert_eq!(&records(ports), before);
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn eleven_eligible_accounts_share_six_slots_only_after_confirmed_close() {
    let (state, ports) = fixture(6, 11);
    let before = records(&ports);
    for n in 1..=6 { activated(&state, n).await; }
    ports.gate_close.store(true, Ordering::SeqCst);
    let mut replacement = Box::pin(activate(State(state.clone()), Json(AccountBody { account_id: id(7) })));
    assert!(futures::poll!(replacement.as_mut()).is_pending());
    assert_eq!(ports.live.lock().unwrap().len(), 6);
    assert!(!ports.live.lock().unwrap().contains_key(&id(7)));
    let snapshot = state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(snapshot.occupied, 6);
    assert!(snapshot.residents.iter().any(|s| s.close_started && s.replacement == Some(id(7))));
    ports.close_gate.add_permits(1);
    replacement.await.unwrap();
    ports.gate_close.store(false, Ordering::SeqCst);
    for n in 8..=11 { activated(&state, n).await; }
    assert_eq!(ports.max_live.load(Ordering::SeqCst), 6);
    assert_eq!(ports.live.lock().unwrap().len(), 6);
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 6);
    assert_eq!(state.serving_accounts.config().active_capacity, 6);
    assert!((1..=11).all(|n| state.serving_accounts.allows(&id(n))));
    let events = ports.events.lock().unwrap();
    let minted: HashSet<_> = events.iter().filter(|(kind, _)| *kind == "mint").map(|(_, id)| *id).collect();
    assert_eq!(minted.len(), 11);
    for n in 7..=11 {
        let at = events.iter().position(|entry| *entry == ("mint", id(n))).unwrap();
        assert_eq!(events[at - 2].0, "close_confirmed", "replacement {n} started too early");
        assert_eq!(events[at - 1], ("status", id(n)));
    }
    drop(events);
    idle_without_writes(&state, &ports, &before);
}

#[tokio::test]
async fn active_leases_reject_park_shrink_and_eligibility_removal_before_close_io() {
    let (state, ports) = fixture(2, 3);
    let first = lease_account(&state, id(1)).await.unwrap();
    let second = lease_account(&state, id(2)).await.unwrap();
    let before = state.serving_accounts.config();
    let before_events = ports.events.lock().unwrap().len();
    assert!(matches!(parked(&state, 1).await, Err(ApiError::PoolBusy(_))));
    assert!(matches!(changed(&state, json!({"active_capacity":1})).await, Err(ApiError::PoolBusy(_))));
    assert!(matches!(changed(&state, json!({"eligible_accounts":[id(2)]})).await, Err(ApiError::PoolBusy(_))));
    assert_eq!(state.serving_accounts.config(), before);
    assert!(ports.events.lock().unwrap()[before_events..].is_empty());
    assert_eq!(ports.setting_writes.load(Ordering::SeqCst), 0);
    drop((first, second));
}

#[tokio::test]
async fn shrinking_parks_only_idle_browsers_and_keeps_saved_health() {
    let (state, ports) = fixture(3, 4);
    let before = records(&ports);
    for n in 1..=3 { activated(&state, n).await; }
    let lease = lease_account(&state, id(1)).await.unwrap();
    let Json(next) = changed(&state, json!({"active_capacity":1})).await.unwrap();
    assert_eq!(next.active_capacity, 1);
    assert_eq!(stored_config(&ports), next);
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 1);
    assert_eq!(slot(&state, id(1)).users, 1);
    assert_eq!(ports.live.lock().unwrap().keys().copied().collect::<HashSet<_>>(), HashSet::from([id(1)]));
    assert_eq!(records(&ports), before);
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
    drop(lease);
}

#[tokio::test]
async fn partial_configuration_preserves_other_fields_and_round_trips_saved_settings() {
    let (state, ports) = fixture(6, 11);
    let Json(first) = changed(&state, json!({"eligible_accounts":[id(2),id(7),id(11)]})).await.unwrap();
    assert_eq!(first.active_capacity, 6);
    let Json(second) = changed(&state, json!({"active_capacity":2})).await.unwrap();
    assert_eq!(second.eligible_accounts, first.eligible_accounts);
    assert_eq!(stored_config(&ports), second);
    assert_eq!(config(State(state.clone())).await.0, second);
    let restored = ServingAccountPolicy::from_config(stored_config(&ports)).unwrap();
    assert!(restored.allows(&id(11)));
    assert!(!restored.allows(&id(1)));
    assert_eq!(restored.residents.capacity().unwrap(), 2);
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn settings_failure_restores_capacity_and_policy_without_reopening_closed_accounts() {
    let (state, ports) = fixture(3, 4);
    changed(&state, json!({"active_capacity":3})).await.unwrap();
    for n in 1..=3 { activated(&state, n).await; }
    let before = records(&ports);
    let previous = state.serving_accounts.config();
    ports.save_failure.store(true, Ordering::SeqCst);
    assert!(matches!(changed(&state, json!({"active_capacity":1,"eligible_accounts":[id(1)]})).await,
        Err(ApiError::Internal(error)) if error.contains("fixture settings failure")));
    assert_eq!(state.serving_accounts.config(), previous);
    assert_eq!(stored_config(&ports), previous);
    let snapshot = state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(snapshot.capacity, 3);
    assert_eq!(snapshot.occupied, 1);
    assert_eq!(ports.live.lock().unwrap().len(), 1);
    assert_eq!(records(&ports), before);
    assert!(state.serving_accounts.allows(&id(4)));
    ports.save_failure.store(false, Ordering::SeqCst);
    activated(&state, 4).await;
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 2);
}

#[tokio::test]
async fn configuration_rejects_invalid_capacity_unknown_and_duplicate_ids_before_mutation() {
    let (state, ports) = fixture(6, 11);
    let previous = state.serving_accounts.config();
    for value in [json!({"active_capacity":0}), json!({"active_capacity":65}),
        json!({"eligible_accounts":[id(12)]}), json!({"eligible_accounts":[id(2),id(2)]})] {
        assert!(matches!(changed(&state, value).await, Err(ApiError::InvalidRequest(_))));
        assert_eq!(state.serving_accounts.config(), previous);
    }
    assert!(serde_json::from_value::<ConfigUpdate>(json!({"unknown":true})).is_err());
    assert!(serde_json::from_value::<ConfigUpdate>(json!({"eligible_accounts":["not-a-uuid"]})).is_err());
    assert!(serde_json::from_value::<AccountBody>(json!({"account_id":id(1),"unknown":true})).is_err());
    assert!(ports.events.lock().unwrap().is_empty());
    assert_eq!(ports.setting_writes.load(Ordering::SeqCst), 0);
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
}

#[tokio::test]
async fn failed_or_mismatched_close_keeps_capacity_until_matching_reconciliation() {
    for mode in [CloseMode::WrongAccount, CloseMode::WrongHost, CloseMode::WrongIncarnation,
        CloseMode::StillResident, CloseMode::Busy, CloseMode::Unconfirmed, CloseMode::ActiveCommand, CloseMode::Error] {
        let (state, ports) = fixture(1, 2);
        activated(&state, 1).await;
        let before = records(&ports);
        *ports.close_mode.lock().unwrap() = mode;
        let result = activate(State(state.clone()), Json(AccountBody { account_id: id(2) })).await;
        assert!(matches!(result, Err(ApiError::PoolBusy(_))), "{mode:?}");
        assert!(slot(&state, id(1)).uncertain, "{mode:?}");
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 1);
        assert!(!ports.live.lock().unwrap().contains_key(&id(2)));
        assert!(state.serving_accounts.residents.try_acquire(id(2), None).is_err());
        *ports.close_mode.lock().unwrap() = CloseMode::Confirm;
        parked(&state, 1).await.unwrap();
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
        activated(&state, 2).await;
        assert_eq!(ports.max_live.load(Ordering::SeqCst), 1);
        idle_without_writes(&state, &ports, &before);
    }
}

#[tokio::test]
async fn busy_park_retains_capacity_and_prior_uncertainty() {
    for uncertain in [false, true] {
        let (state, ports) = fixture(1, 2);
        activated(&state, 1).await;
        if uncertain {
            let lease = lease_account(&state, id(1)).await.unwrap();
            lease.mark_unconfirmed().unwrap();
            drop(lease);
        }
        *ports.close_mode.lock().unwrap() = CloseMode::BusyResident;
        let outcome = state.serving_accounts.park_managed_observed(
            state.browser.as_ref(), id(1)).await;
        assert!(outcome.result.is_err());
        assert!(outcome.cleanup_confirmed);
        assert_eq!(slot(&state, id(1)).uncertain, uncertain);
        assert_eq!(slot(&state, id(1)).replacement, None);
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 1);
        assert!(ports.live.lock().unwrap().contains_key(&id(1)));
        assert!(!ports.live.lock().unwrap().contains_key(&id(2)));
    }
}

#[tokio::test]
async fn dropping_inflight_park_or_replacement_retains_an_uncertain_slot() {
    for replacement in [false, true] {
        let (state, ports) = fixture(1, 2);
        activated(&state, 1).await;
        ports.gate_close.store(true, Ordering::SeqCst);
        let mut closing = Box::pin(async {
            if replacement {
                activate(State(state.clone()), Json(AccountBody { account_id: id(2) })).await
            } else { parked(&state, 1).await }
        });
        assert!(futures::poll!(closing.as_mut()).is_pending());
        assert!(slot(&state, id(1)).close_started);
        drop(closing);
        assert!(slot(&state, id(1)).uncertain);
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 1);
        assert_eq!(ports.live.lock().unwrap().len(), 1);
        assert!(!ports.live.lock().unwrap().contains_key(&id(2)));
        ports.gate_close.store(false, Ordering::SeqCst);
        parked(&state, 1).await.unwrap();
        activated(&state, 2).await;
        assert_eq!(ports.max_live.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn status_failure_before_close_cancels_reservation_without_marking_browser_uncertain() {
    for mode in [StatusMode::Busy, StatusMode::WrongAccount, StatusMode::Error] {
        let (state, ports) = fixture(1, 2);
        activated(&state, 1).await;
        *ports.status_mode.lock().unwrap() = mode;
        assert!(matches!(activate(State(state.clone()), Json(AccountBody { account_id: id(2) })).await,
            Err(ApiError::PoolBusy(_))));
        let resident = slot(&state, id(1));
        assert!(!resident.uncertain);
        assert!(!resident.close_started);
        assert_eq!(resident.replacement, None);
        assert!(!ports.events.lock().unwrap().iter().any(|event| event.0 == "close_started"));
        *ports.status_mode.lock().unwrap() = StatusMode::Ready;
        activated(&state, 2).await;
        assert_eq!(ports.max_live.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn a_failed_shrink_keeps_old_configuration_and_the_uncertain_slot() {
    let (state, ports) = fixture(2, 2);
    activated(&state, 1).await;
    activated(&state, 2).await;
    let previous = state.serving_accounts.config();
    *ports.close_mode.lock().unwrap() = CloseMode::Error;
    assert!(matches!(changed(&state, json!({"active_capacity":1})).await, Err(ApiError::PoolBusy(_))));
    assert_eq!(state.serving_accounts.config(), previous);
    let snapshot = state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(snapshot.capacity, 2);
    assert_eq!(snapshot.occupied, 2);
    assert_eq!(snapshot.residents.iter().filter(|s| s.uncertain).count(), 1);
    assert_eq!(ports.setting_writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn replacement_prefers_a_cooling_idle_account_without_resetting_its_health() {
    let (state, ports) = fixture(2, 3);
    activated(&state, 1).await;
    activated(&state, 2).await;
    ports.records.lock().unwrap()[1].cooling_until_ms = Some(framer_core::rotation::now_ms() + 60_000);
    let before = records(&ports);
    activated(&state, 3).await;
    let live = ports.live.lock().unwrap().keys().copied().collect::<HashSet<_>>();
    assert_eq!(live, HashSet::from([id(1), id(3)]));
    idle_without_writes(&state, &ports, &before);
}

#[tokio::test]
async fn management_rejects_active_work_and_protects_its_account_until_guard_drops() {
    let (state, ports) = fixture(1, 1);
    activated(&state, 1).await;
    let lease = lease_account(&state, id(1)).await.unwrap();
    assert!(matches!(management(&state, id(1), false).await, Err(ApiError::PoolBusy(_))));
    assert!(matches!(management(&state, id(1), true).await, Err(ApiError::PoolBusy(_))));
    assert_eq!(slot(&state, id(1)).users, 1);
    drop(lease);
    let guard = management(&state, id(1), true).await.unwrap();
    assert_eq!(slot(&state, id(1)).users, 1);
    let mut parking = Box::pin(parked(&state, 1));
    match futures::poll!(parking.as_mut()) {
        std::task::Poll::Pending | std::task::Poll::Ready(Err(ApiError::PoolBusy(_))) => {},
        _ => panic!("management permitted its account to be parked"),
    }
    assert!(!ports.events.lock().unwrap().iter().any(|event| event.0 == "close_started"));
    drop(parking);
    drop(guard);
    parked(&state, 1).await.unwrap();
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
}

#[derive(Clone, Copy, Debug)]
enum Route { Free, Native, Compat }
const ROUTES: [Route; 3] = [Route::Free, Route::Native, Route::Compat];

async fn streaming_response(state: &AppState, route: Route) -> axum::response::Response {
    handler_response(state, route, true).await.unwrap()
}

async fn handler_response(state: &AppState, route: Route, stream: bool)
    -> Result<axum::response::Response, ApiError> {
    handler_response_with_session(state, route, stream, None).await
}

async fn handler_response_with_session(state: &AppState, route: Route, stream: bool, session: Option<&str>)
    -> Result<axum::response::Response, ApiError> {
    let mut headers = HeaderMap::new();
    // These pool tests exercise automatic selection and account rotation.
    // Explicit-account requests have their own no-repick fixtures.
    if let Some(session) = session { headers.insert("x-framerize-session", session.parse().unwrap()); }
    match route {
        Route::Free => crate::free_complete(state.clone(), &headers, ClientDialect::AnthropicMessages,
            json!({"model":"claude-opus-5-5","stream":stream,
                "messages":[{"role":"user","content":"fixture"}]})).await,
        Route::Native => crate::native_complete(state.clone(), &headers, ClientDialect::OpenAiResponses,
            json!({"model":"gpt-6-astra","input":"fixture","stream":stream}), stream, false, 64).await,
        Route::Compat => crate::compat_forward(state.clone(), "openai-responses", Some(id(1).0.to_string()), session.map(str::to_owned),
            json!({"model":"fixture","input":"fixture","stream":stream})).await,
    }
}

async fn assert_stream_residency(state: &AppState) {
    assert!(slot(state, id(1)).users > 0);
    assert!(matches!(parked(state, 1).await, Err(ApiError::PoolBusy(_))));
    assert!(matches!(changed(state, json!({"eligible_accounts":[]})).await, Err(ApiError::PoolBusy(_))));
}

fn assert_repick_chats(ports: &Ports, explicit: Option<&str>, before: &[String; 2]) {
    let chats = ports.observed_chats.lock().unwrap();
    assert_eq!(chats.iter().map(|(account, _)| *account).collect::<Vec<_>>(), vec![id(1), id(2)]);
    for (index, (account, chat)) in chats.iter().enumerate() {
        if let Some(explicit) = explicit {
            assert_eq!(chat, explicit);
        } else {
            // Accept a five-hour session-window rollover during this fixture.
            assert!(chat == &before[index] || chat == &crate::stable_session(account));
        }
    }
    if explicit.is_none() { assert_ne!(chats[0].1, chats[1].1); }
}

#[tokio::test]
async fn all_stream_handlers_retain_residency_through_chain_and_unread_completed_body() {
    for route in ROUTES {
        let (mut state, ports) = fixture(1, 2);
        state.runtime_mut().expect("unique fixture runtime").gate = one_slot_gate();
        let response = streaming_response(&state, route).await;
        until(|| ports.request_started.load(Ordering::SeqCst) == 1).await;
        assert_stream_residency(&state).await;
        if matches!(route, Route::Compat) {
            assert!(state.gate.acquire_now("team-1:fixture").await.is_none());
        }
        ports.request_gate.add_permits(1);
        until(|| ports.request_steps.load(Ordering::SeqCst) == 1).await;
        assert_stream_residency(&state).await;
        ports.request_gate.add_permits(1);
        let response = if matches!(route, Route::Native) {
            crate::response_output::pump_until_producer_returns(response,
                || ports.request_drops.load(Ordering::SeqCst) == 1).await
        } else { response };
        until(|| ports.request_drops.load(Ordering::SeqCst) == 1).await;
        assert_eq!(ports.request_finished.load(Ordering::SeqCst), 1, "{route:?}");
        assert_stream_residency(&state).await;
        if matches!(route, Route::Compat) {
            assert!(state.gate.acquire_now("team-1:fixture").await.is_none(), "unread body released its gate");
        }
        assert_eq!(ports.setting_writes.load(Ordering::SeqCst), 0);
        ports.response_gate.add_permits(2);
        let body = tokio::time::timeout(Duration::from_secs(2), to_bytes(response.into_body(), 65_536))
            .await.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&body).contains("fixture"), "{route:?}");
        until(|| slot(&state, id(1)).users == 0).await;
        if matches!(route, Route::Compat) {
            assert!(state.gate.acquire_now("team-1:fixture").await.is_some(), "finished body leaked its gate");
        }
        parked(&state, 1).await.unwrap();
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
    }
}

#[tokio::test]
async fn dropping_never_polled_bodies_releases_leases_even_before_producer_first_poll() {
    for route in ROUTES {
        let (state, ports) = fixture(1, 1);
        let response = streaming_response(&state, route).await;
        assert!(slot(&state, id(1)).users > 0);
        drop(response);
        until(|| slot(&state, id(1)).users == 0).await;
        assert_eq!(ports.request_finished.load(Ordering::SeqCst), 0, "{route:?}");
        assert_eq!(ports.request_drops.load(Ordering::SeqCst), ports.request_started.load(Ordering::SeqCst));
        parked(&state, 1).await.unwrap();
    }
}

#[tokio::test]
async fn retry_releases_previous_account_and_keeps_replacement_until_unread_body_drops() {
    for route in [Route::Free, Route::Native] {
      for requested_session in [None, Some("fixture-explicit-chat")] {
        let (state, ports) = fixture(1, 2);
        ports.fail_first_account.store(true, Ordering::SeqCst);
        let before_chats = [crate::stable_session(&id(1)), crate::stable_session(&id(2))];
        let response = handler_response_with_session(&state, route, true, requested_session).await.unwrap();
        until(|| ports.request_started.load(Ordering::SeqCst) == 2).await;
        assert_repick_chats(&ports, requested_session, &before_chats);
        let snapshot = state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.occupied, 1);
        assert_eq!(snapshot.residents[0].account, id(2));
        assert!(snapshot.residents[0].users > 0);
        assert_eq!(ports.max_live.load(Ordering::SeqCst), 1);
        assert!(matches!(parked(&state, 2).await, Err(ApiError::PoolBusy(_))));
        ports.request_gate.add_permits(2);
        let response = if matches!(route, Route::Native) {
            crate::response_output::pump_until_producer_returns(response,
                || ports.request_drops.load(Ordering::SeqCst) == 2).await
        } else { response };
        until(|| ports.request_drops.load(Ordering::SeqCst) == 2).await;
        assert_eq!(ports.request_finished.load(Ordering::SeqCst), 1);
        assert!(slot(&state, id(2)).users > 0, "{route:?}");
        drop(response);
        until(|| slot(&state, id(2)).users == 0).await;
        parked(&state, 2).await.unwrap();
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
      }
    }
}

// Required safety regressions from transport review 916f156c. The reviewed
// implementation did not satisfy these admission and management contracts.
#[tokio::test]
async fn unknown_account_rejection_does_not_evict_any_of_six_resident_browsers() {
    for through_run in [false, true] {
        let (state, ports) = fixture(6, 11);
        for n in 1..=6 { activated(&state, n).await; }
        let before = records(&ports);
        let live = ports.live.lock().unwrap().clone();
        let events = ports.events.lock().unwrap().clone();
        if through_run {
            let response = crate::run(State(state.clone()), Json(crate::RunBody {
                account_id: id(999), prompt: "fixture".into(), model: None, chat_session: None,
            })).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
            let body = to_bytes(response.into_body(), 4096).await.unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(),
                json!({"error":{"code":"invalid_request","message":"account not found"}}));
        } else {
            assert!(matches!(lease_account(&state, id(999)).await,
                Err(ApiError::InvalidRequest(_))));
        }
        assert_eq!(*ports.live.lock().unwrap(), live);
        assert_eq!(*ports.events.lock().unwrap(), events);
        let snapshot = state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.occupied, 6);
        assert_eq!(snapshot.residents.iter().map(|slot| slot.account).collect::<HashSet<_>>(),
            (1..=6).map(id).collect::<HashSet<_>>());
        idle_without_writes(&state, &ports, &before);
    }
}

#[tokio::test]
async fn paused_account_management_does_not_block_an_unrelated_account_admission() {
    let (state, ports) = fixture(2, 2);
    activated(&state, 1).await;
    let guard = management(&state, id(1), true).await.unwrap();
    let unrelated = tokio::time::timeout(Duration::from_secs(1), lease_account(&state, id(2)))
        .await.expect("unrelated admission waited on another account's management operation")
        .unwrap();
    assert_eq!(slot(&state, id(1)).users, 1);
    assert_eq!(slot(&state, id(2)).users, 1);
    assert!(!ports.events.lock().unwrap().iter().any(|event| event.0 == "close_started"));
    drop(unrelated);
    assert_eq!(slot(&state, id(1)).users, 1);
    drop(guard);
    assert_eq!(slot(&state, id(1)).users, 0);
}

#[tokio::test]
async fn cooldown_recorded_during_activation_admission_prevents_browser_minting() {
    let (state, ports) = fixture(1, 2);
    activated(&state, 1).await;
    ports.gate_close.store(true, Ordering::SeqCst);
    let mut activating = Box::pin(activate(State(state.clone()), Json(AccountBody { account_id: id(2) })));
    assert!(futures::poll!(activating.as_mut()).is_pending());
    assert!(slot(&state, id(1)).close_started);
    ports.records.lock().unwrap()[1].cooling_until_ms = Some(framer_core::rotation::now_ms() + 60_000);
    let before = records(&ports);
    ports.close_gate.add_permits(1);
    assert!(matches!(activating.await, Err(ApiError::AccountCooling { .. })));
    assert!(!ports.live.lock().unwrap().contains_key(&id(2)));
    assert!(!ports.events.lock().unwrap().contains(&("mint", id(2))));
    idle_without_writes(&state, &ports, &before);
}

async fn assert_not_admitted(state: &AppState, account: AccountId) {
    match tokio::time::timeout(Duration::from_millis(50), lease_account(state, account)).await {
        Err(_) | Ok(Err(ApiError::PoolBusy(_) | ApiError::ServingCapacity(_))) => {},
        Ok(Ok(_)) => panic!("account {account:?} was admitted during a conflicting operation"),
        Ok(Err(error)) => panic!("unexpected admission error: {error:?}"),
    }
}

#[tokio::test]
async fn existing_account_admission_proceeds_during_unrelated_park_or_replacement_close() {
    for replacement in [false, true] {
        let (state, ports) = fixture(2, 3);
        activated(&state, 1).await;
        activated(&state, 2).await;
        ports.gate_close.store(true, Ordering::SeqCst);
        let mut closing = Box::pin(async {
            if replacement { activate(State(state.clone()), Json(AccountBody { account_id: id(3) })).await }
            else { parked(&state, 1).await }
        });
        assert!(futures::poll!(closing.as_mut()).is_pending());
        assert!(slot(&state, id(1)).close_started);
        let unrelated = tokio::time::timeout(Duration::from_secs(1), lease_account(&state, id(2)))
            .await.expect("unrelated resident was blocked by browser close I/O").unwrap();
        assert_eq!(slot(&state, id(2)).users, 1);
        assert!(!ports.live.lock().unwrap().contains_key(&id(3)));
        ports.close_gate.add_permits(1);
        closing.await.unwrap();
        assert_eq!(slot(&state, id(2)).users, 1);
        assert_eq!(ports.max_live.load(Ordering::SeqCst), 2);
        drop(unrelated);
    }
}

#[tokio::test]
async fn configuration_closes_preserve_unaffected_work_but_freeze_new_or_excluded_admission() {
    let (state, ports) = fixture(4, 5);
    for n in 1..=4 { activated(&state, n).await; }
    let previous = state.serving_accounts.config();
    let before = records(&ports);
    ports.gate_close.store(true, Ordering::SeqCst);
    let mut changing = Box::pin(changed(&state,
        json!({"active_capacity":3,"eligible_accounts":[id(2),id(4),id(5)]})));
    assert!(futures::poll!(changing.as_mut()).is_pending());
    let unaffected = tokio::time::timeout(Duration::from_secs(1), lease_account(&state, id(2)))
        .await.expect("configuration blocked an unaffected resident").unwrap();
    assert_not_admitted(&state, id(3)).await;
    assert_not_admitted(&state, id(5)).await;
    assert_eq!(state.serving_accounts.config(), previous);
    assert_eq!(ports.setting_writes.load(Ordering::SeqCst), 0);
    ports.close_gate.add_permits(1);
    assert!(futures::poll!(changing.as_mut()).is_pending());
    assert_eq!(ports.events.lock().unwrap().iter().filter(|event| event.0 == "close_confirmed").count(), 1);
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 3);
    assert_eq!(slot(&state, id(2)).users, 1);
    assert_not_admitted(&state, id(1)).await;
    assert_not_admitted(&state, id(3)).await;
    assert_not_admitted(&state, id(5)).await;
    let second = tokio::time::timeout(Duration::from_secs(1), lease_account(&state, id(2)))
        .await.expect("later configuration close blocked existing work").unwrap();
    assert_eq!(slot(&state, id(2)).users, 2);
    ports.close_gate.add_permits(1);
    let Json(next) = changing.await.unwrap();
    assert_eq!(next.active_capacity, 3);
    assert_eq!(stored_config(&ports), next);
    assert!(matches!(lease_account(&state, id(1)).await, Err(ApiError::ServingCapacity(_))));
    assert!(matches!(lease_account(&state, id(3)).await, Err(ApiError::ServingCapacity(_))));
    activated(&state, 5).await;
    assert_eq!(ports.live.lock().unwrap().keys().copied().collect::<HashSet<_>>(),
        HashSet::from([id(2), id(4), id(5)]));
    assert_eq!(slot(&state, id(2)).users, 2);
    drop(second);
    drop(unaffected);
    idle_without_writes(&state, &ports, &before);
}

#[tokio::test]
async fn account_management_blocks_its_target_while_other_accounts_continue() {
    for browser in [false, true] {
        let (state, ports) = fixture(2, 2);
        activated(&state, 1).await;
        let guard = management(&state, id(1), browser).await.unwrap();
        assert_not_admitted(&state, id(1)).await;
        let unrelated = tokio::time::timeout(Duration::from_secs(1), lease_account(&state, id(2)))
            .await.expect("management blocked an unrelated account").unwrap();
        let mut targeted = Box::pin(activate(State(state.clone()), Json(AccountBody { account_id: id(1) })));
        match futures::poll!(targeted.as_mut()) {
            std::task::Poll::Pending | std::task::Poll::Ready(Err(ApiError::PoolBusy(_))) => {},
            _ => panic!("targeted activation did not honor account management"),
        }
        drop(targeted);
        assert_eq!(slot(&state, id(1)).users, usize::from(browser));
        assert_eq!(ports.events.lock().unwrap().iter().filter(|event| **event == ("mint", id(1))).count(), 1);
        assert_eq!(slot(&state, id(2)).users, 1);
        drop(guard);
        activated(&state, 1).await;
        drop(unrelated);
    }
}

#[tokio::test]
async fn wave_pause_started_during_activation_admission_prevents_browser_minting() {
    let (state, ports) = fixture(1, 2);
    activated(&state, 1).await;
    let before = records(&ports);
    ports.gate_close.store(true, Ordering::SeqCst);
    let mut activating = Box::pin(activate(State(state.clone()), Json(AccountBody { account_id: id(2) })));
    assert!(futures::poll!(activating.as_mut()).is_pending());
    assert!(slot(&state, id(1)).close_started);
    state.wave.note_refusal("fixture-wave-a");
    state.wave.note_refusal("fixture-wave-b");
    assert!(state.wave.check().is_some());
    ports.close_gate.add_permits(1);
    assert!(matches!(activating.await, Err(ApiError::Cooling { .. })));
    assert!(!ports.live.lock().unwrap().contains_key(&id(2)));
    assert!(!ports.events.lock().unwrap().contains(&("mint", id(2))));
    idle_without_writes(&state, &ports, &before);
}

fn import_body(proxy: Option<String>) -> crate::ImportBody {
    crate::ImportBody {
        account_id: id(2), proxy, session_cookie: "session=fixture-only".into(),
        expected_framer_user_id: None,
        email: None, alias: None, project_id: Some("project-2".into()), team_id: Some("team-2".into()),
    }
}

#[tokio::test]
async fn excluded_import_registers_saved_or_supplied_proxy_for_later_runtime_activation() {
    for supplied in [None, Some("http://import-route.invalid:8080".to_string())] {
        let (state, ports) = fixture(2, 2);
        changed(&state, json!({"eligible_accounts":[id(1)]})).await.unwrap();
        ports.allow_import.store(true, Ordering::SeqCst);
        let expected = supplied.clone().or_else(|| account(2).proxy);
        let Json(result) = crate::import_account(State(state.clone()), Json(import_body(supplied))).await.unwrap();
        assert_eq!(result["status"], "imported");
        assert_eq!(result["serving_allowed"], false);
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
        assert!(ports.live.lock().unwrap().is_empty());
        assert_eq!(ports.events.lock().unwrap().as_slice(), &[("register_proxy", id(2))]);
        assert_eq!(ports.registered_proxies.lock().unwrap().get(&id(2)), Some(&expected));
        assert_eq!(ports.saved_sessions.lock().unwrap().len(), 1);
        assert_eq!(ports.records.lock().unwrap()[1].proxy, expected);
        assert_eq!(ports.records.lock().unwrap()[1].castle_device_id, "device-2");
        let after_import = records(&ports);
        changed(&state, json!({"eligible_accounts":"all"})).await.unwrap();
        assert!(ports.live.lock().unwrap().is_empty());
        activated(&state, 2).await;
        assert_eq!(ports.mint_routes.lock().unwrap().as_slice(), &[(id(2), expected.clone())]);
        parked(&state, 2).await.unwrap();
        activated(&state, 2).await;
        assert_eq!(ports.mint_routes.lock().unwrap().as_slice(),
            &[(id(2), expected.clone()), (id(2), expected)]);
        assert_eq!(records(&ports), after_import);
    }
}

#[tokio::test]
async fn import_warmup_rechecks_cooldown_after_waiting_for_residency() {
    let (state, ports) = fixture(1, 2);
    activated(&state, 1).await;
    ports.allow_import.store(true, Ordering::SeqCst);
    ports.gate_close.store(true, Ordering::SeqCst);
    crate::import_account(State(state.clone()), Json(import_body(None))).await.unwrap();
    until(|| ports.events.lock().unwrap().contains(&("close_started", id(1)))).await;
    ports.records.lock().unwrap()[1].cooling_until_ms = Some(framer_core::rotation::now_ms() + 60_000);
    let before = records(&ports);
    ports.close_gate.add_permits(1);
    until(|| ports.events.lock().unwrap().contains(&("close_confirmed", id(1)))
        && state.serving_accounts.residents.snapshot().unwrap().residents.iter()
            .all(|slot| slot.users == 0 && !slot.close_started)).await;
    assert!(!ports.mint_routes.lock().unwrap().iter().any(|(account, _)| *account == id(2)));
    assert!(!ports.live.lock().unwrap().contains_key(&id(2)));
    assert_eq!(records(&ports), before);
}

#[tokio::test]
async fn prewarm_rechecks_cooldown_after_waiting_for_residency() {
    let (state, ports) = fixture(1, 2);
    activated(&state, 1).await;
    ports.records.lock().unwrap()[0].cooling_until_ms = Some(framer_core::rotation::now_ms() + 60_000);
    ports.gate_close.store(true, Ordering::SeqCst);
    let mut warming = Box::pin(super::prewarm_accounts(state.clone()));
    assert!(futures::poll!(warming.as_mut()).is_pending());
    assert!(slot(&state, id(1)).close_started);
    ports.records.lock().unwrap()[1].cooling_until_ms = Some(framer_core::rotation::now_ms() + 60_000);
    let before = records(&ports);
    ports.close_gate.add_permits(1);
    warming.await;
    assert!(!ports.mint_routes.lock().unwrap().iter().any(|(account, _)| *account == id(2)));
    assert!(!ports.live.lock().unwrap().contains_key(&id(2)));
    idle_without_writes(&state, &ports, &before);
}

#[tokio::test]
async fn handlers_recheck_account_and_wave_after_waiting_for_the_model_gate() {
    for route in ROUTES {
        for stream in [false, true] {
            for wave in [false, true] {
                let (mut state, ports) = fixture(1, 1);
                state.runtime_mut().expect("unique fixture runtime").gate = one_slot_gate();
                let model = match route {
                    Route::Free => "claude-opus-5-5", Route::Native => "gpt-6-astra", Route::Compat => "fixture",
                };
                let key = format!("team-1:{model}");
                let permit = state.gate.acquire(&key).await;
                let mut request = Box::pin(handler_response(&state, route, stream));
                assert!(futures::poll!(request.as_mut()).is_pending());
                assert!(slot(&state, id(1)).users > 0);
                assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
                assert!(ports.events.lock().unwrap().iter().all(|event| event.0 == "status"));
                if wave {
                    state.wave.note_refusal("fixture-gate-a");
                    state.wave.note_refusal("fixture-gate-b");
                    assert!(state.wave.check().is_some());
                } else {
                    ports.records.lock().unwrap()[0].cooling_until_ms = Some(framer_core::rotation::now_ms() + 60_000);
                }
                drop(permit);
                assert_cooling_response(request.await, wave).await;
                assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
                assert!(ports.events.lock().unwrap().iter().all(|event| event.0 == "status"));
                assert_eq!(slot(&state, id(1)).users, 0);
            }
        }
    }
}

#[tokio::test]
async fn failed_or_cancelled_configuration_does_not_make_untouched_browsers_uncertain() {
    for cancel in [false, true] {
        let (state, ports) = fixture(3, 3);
        for n in 1..=3 { activated(&state, n).await; }
        let event_start = ports.events.lock().unwrap().len();
        let before = records(&ports);
        let previous = state.serving_accounts.config();
        ports.gate_close.store(cancel, Ordering::SeqCst);
        *ports.close_mode.lock().unwrap() = CloseMode::Error;
        let mut change = Box::pin(changed(&state, json!({"active_capacity":1,"eligible_accounts":[id(2)]})));
        if cancel {
            assert!(futures::poll!(change.as_mut()).is_pending());
            drop(change);
        } else { assert!(matches!(change.await, Err(ApiError::PoolBusy(_)))); }
        let events = ports.events.lock().unwrap()[event_start..].to_vec();
        let touched = events.iter().find(|event| event.0 == "close_started").unwrap().1;
        let untouched = if touched == id(1) { id(3) } else { id(1) };
        assert!(slot(&state, touched).uncertain);
        assert!(!slot(&state, untouched).uncertain);
        assert!(!slot(&state, untouched).close_started);
        assert!(!events.contains(&("status", untouched)));
        assert!(!events.contains(&("close_started", untouched)));
        assert_eq!(state.serving_accounts.config(), previous);
        assert_eq!(ports.setting_writes.load(Ordering::SeqCst), 0);
        idle_without_writes(&state, &ports, &before);
    }
}

#[tokio::test]
async fn automatic_selection_uses_unmanaged_residents_before_busy_or_unloaded_accounts() {
    let (state, ports) = fixture(2, 3);
    activated(&state, 1).await;
    activated(&state, 2).await;
    let guard = management(&state, id(1), false).await.unwrap();
    for _ in 0..8 {
        let chosen = state.serving_accounts.select(ports.records.lock().unwrap().clone(), None).unwrap();
        assert_eq!(chosen.id, id(2));
        let fresh = state.serving_accounts.select_fresh(ports.records.lock().unwrap().clone(), &[]).unwrap();
        assert_eq!(fresh.id, id(2));
    }
    drop(guard);
}

#[tokio::test]
async fn configuration_revalidates_ids_after_a_stale_store_snapshot_or_serializes_the_removal() {
    let (state, ports) = fixture(2, 2);
    ports.pause_list.store(true, Ordering::SeqCst);
    let mut change = Box::pin(changed(&state, json!({"eligible_accounts":[id(2)]})));
    assert!(futures::poll!(change.as_mut()).is_pending());
    assert_eq!(ports.list_captured.load(Ordering::SeqCst), 1);
    let mut removal = Box::pin(management(&state, id(2), false));
    let prior = state.serving_accounts.config();
    let guard = match futures::poll!(removal.as_mut()) {
        std::task::Poll::Ready(Ok(guard)) => Some(guard),
        std::task::Poll::Pending | std::task::Poll::Ready(Err(ApiError::PoolBusy(_))) => None,
        _ => panic!("unexpected removal admission result"),
    };
    drop(removal);
    if let Some(guard) = guard {
        // A concurrent remover completed while the old list result was delayed.
        ports.records.lock().unwrap().retain(|account| account.id != id(2));
        drop(guard);
        ports.list_gate.add_permits(1);
        assert!(matches!(change.await, Err(ApiError::InvalidRequest(_))));
        assert_eq!(state.serving_accounts.config(), prior);
        assert_eq!(ports.setting_writes.load(Ordering::SeqCst), 0);
    } else {
        // Reading under operation is also valid: removal follows publication.
        ports.list_gate.add_permits(1);
        change.await.unwrap();
        let guard = management(&state, id(2), false).await.unwrap();
        assert!(ports.records.lock().unwrap().iter().any(|account| account.id == id(2)));
        ports.records.lock().unwrap().retain(|account| account.id != id(2));
        drop(guard);
    }
    assert!(ports.events.lock().unwrap().is_empty());
}

fn one_slot_gate() -> Arc<framer_core::gate::ConcurrencyGate> {
    Arc::new(framer_core::gate::ConcurrencyGate::with_config(framer_core::gate::GateConfig {
        initial: 1, min: 1, max: 1, recover_ms: 60_000, relax_ms: 60_000, min_interval_ms: 0,
    }))
}

#[tokio::test]
async fn nonstream_overload_retry_does_not_wait_on_its_own_original_gate_permit() {
    let (mut state, ports) = fixture(1, 1);
    state.runtime_mut().expect("unique fixture runtime").gate = one_slot_gate();
    ports.records.lock().unwrap()[0].current_team = Some(TeamId("overload-fixture-team".into()));
    ports.free_overload_once.store(true, Ordering::SeqCst);
    let key = "overload-fixture-team:claude-opus-5-5";
    let mut request = Box::pin(handler_response(&state, Route::Free, false));
    assert!(futures::poll!(request.as_mut()).is_pending());
    if ports.request_started.load(Ordering::SeqCst) == 1 {
        // No timer is advanced: inspect the permit while the retry is suspended.
        let available = state.gate.acquire_now(key).await;
        assert!(available.is_some(), "retry retained its original gate permit");
        drop(available);
        drop(request);
    } else {
        assert_eq!(ports.request_started.load(Ordering::SeqCst), 2);
        ports.request_gate.add_permits(2);
        request.await.unwrap();
    }
    assert_eq!(slot(&state, id(1)).users, 0);
}

#[tokio::test]
async fn account_repicks_wait_on_destination_gate_and_recheck_health_before_invocation() {
    for route in [Route::Free, Route::Native] {
        for stream in [false, true] {
            for wave in [false, true] {
                let (mut state, ports) = fixture(1, 2);
                state.runtime_mut().expect("unique fixture runtime").gate = one_slot_gate();
                ports.fail_first_account.store(true, Ordering::SeqCst);
                let model = if matches!(route, Route::Free) { "claude-opus-5-5" } else { "gpt-6-astra" };
                let gate_key = format!("team-2:{model}");
                let held = state.gate.acquire(&gate_key).await;
                let mut request = Box::pin(handler_response(&state, route, stream));
                let response = if stream { Some(request.as_mut().await.unwrap()) } else {
                    assert!(futures::poll!(request.as_mut()).is_pending());
                    None
                };
                until(|| state.serving_accounts.residents.snapshot().unwrap().residents.iter()
                    .any(|slot| slot.account == id(2) && slot.users > 0)).await;
                tokio::task::yield_now().await;
                assert_eq!(ports.request_started.load(Ordering::SeqCst), 1, "{route:?}, stream={stream}");
                assert!(!ports.mint_routes.lock().unwrap().iter().any(|(account, _)| *account == id(2)));
                if wave {
                    state.wave.note_refusal("fixture-repick-a");
                    state.wave.note_refusal("fixture-repick-b");
                    assert!(state.wave.check().is_some());
                } else {
                    ports.records.lock().unwrap()[1].cooling_until_ms = Some(framer_core::rotation::now_ms() + 60_000);
                }
                drop(held);
                if let Some(response) = response {
                    let mut body = Box::pin(to_bytes(response.into_body(), 65_536));
                    if wave {
                        let body = tokio::time::timeout(Duration::from_secs(2), body)
                            .await.unwrap().unwrap();
                        assert!(!body.is_empty());
                    } else {
                        // A local cooldown now keeps an open stream pending.
                        // Dropping it must still release every permit.
                        assert!(tokio::time::timeout(Duration::from_millis(20), body.as_mut())
                            .await.is_err());
                        drop(body);
                    }
                } else { assert_cooling_response(request.await, wave).await; }
                assert_eq!(ports.request_started.load(Ordering::SeqCst), 1);
                assert!(!ports.mint_routes.lock().unwrap().iter().any(|(account, _)| *account == id(2)));
                until(|| state.serving_accounts.residents.snapshot().unwrap().residents.iter()
                    .all(|slot| slot.users == 0)).await;
            }
        }
    }
}

async fn assert_cooling_response(result: Result<axum::response::Response, ApiError>, wave: bool) {
    let response = result.unwrap_or_else(axum::response::IntoResponse::into_response);
    assert_eq!(response.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error"]["code"], if wave { "framerize_cooling" } else { "framerize_account_cooling" });
}
