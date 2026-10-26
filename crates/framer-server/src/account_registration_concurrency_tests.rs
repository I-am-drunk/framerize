//! Inert handler races. No browser, external service or filesystem operations.
use super::*;
use framer_core::convert::{AssembledResponse, AssembledResponses, ChainEvent};
use framer_core::ids::{FramerUserId, ProjectId, TeamId};
use framer_core::pipeline::InferenceResult;
use framer_core::port::{CastleToken, ChainConfig, ProxyResponse, Session, WorkspaceSpec};
use framer_core::workspace::CreditSnapshot;
use std::collections::BTreeMap;
use std::sync::{Mutex, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(3);
const SHORT: Duration = Duration::from_millis(40);
const EMAIL: &str = "shared@example.invalid";

type Outcome = pool_control::CleanupOutcome<Json<Value>>;
type Task = tokio::task::JoinHandle<Outcome>;
type Joined = Result<Outcome, tokio::task::JoinError>;

async fn settle(task: &mut Task) -> Joined {
    match timeout(WAIT, &mut *task).await {
        Ok(result) => result,
        Err(_) => {
            task.abort();
            task.await
        }
    }
}

struct Competition {
    ports: Arc<Ports>,
    first: Outcome,
    second: Outcome,
    second_waited: bool,
    midway: Vec<ManagedAccount>,
    saved_midway: BTreeMap<String, String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind { Login, Import }
#[derive(Clone, Copy, PartialEq)]
enum Hold { Save, Grant, Publish, Usage }
#[derive(Clone, Copy, PartialEq)]
enum Failure { None, Save, Publish }

struct Ports {
    records: Mutex<Vec<ManagedAccount>>,
    saved: Mutex<BTreeMap<String, String>>,
    second_previous_session: Mutex<Option<String>>,
    calls: Mutex<Vec<(u128, &'static str)>>,
    hold: Hold, failure: Failure, distinct: bool,
    held: AtomicBool,
    entered: Notify, release: Notify, second_started: Notify,
}

fn id(n: u128) -> AccountId { AccountId(uuid::Uuid::from_u128(n)) }
fn cookie(n: u128) -> String { format!("session=fixture-{n}") }
fn number(session: &Session) -> u128 {
    session.cookie_header.strip_prefix("session=fixture-").unwrap().parse().unwrap()
}

impl Ports {
    fn user(&self, n: u128) -> String {
        if self.distinct { format!("user-{n}") } else { "shared-user".into() }
    }
    fn team(&self, n: u128) -> String {
        if self.distinct { format!("team-{n}") } else { "shared-team".into() }
    }
    fn identity(&self, n: u128) -> Value {
        serde_json::json!({"id": self.user(n), "spaces": [
            {"scope": "organization", "ownerId": self.team(n)}]})
    }
    fn call(&self, n: u128, name: &'static str) {
        self.calls.lock().unwrap().push((n, name));
    }
    async fn hold(&self, n: u128, stage: Hold) {
        if n == 1 && self.hold == stage && !self.held.swap(true, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }
}

fn fixture(kinds: [Kind; 2], hold: Hold, failure: Failure, distinct: bool)
    -> (AppState, Arc<Ports>) {
    let ports = Arc::new(Ports {
        records: Mutex::new(Vec::new()), saved: Mutex::new(BTreeMap::new()),
        second_previous_session: Mutex::new(None),
        calls: Mutex::new(Vec::new()), hold, failure, distinct,
        held: AtomicBool::new(false), entered: Notify::new(),
        release: Notify::new(), second_started: Notify::new(),
    });
    // Only login callers need residency. Imports must never spawn prewarming.
    let allowed = kinds.into_iter().enumerate().filter_map(|(n, kind)|
        (kind == Kind::Login).then_some(id(n as u128 + 1))).collect();
    let policy = ServingAccountPolicy::from_config(PoolConfig {
        eligible_accounts: EligibleAccounts::Only(allowed), ..PoolConfig::default()
    }).unwrap();
    let state = AppState::fixture(ports.clone(), ports.clone(), ports.clone(),
        ports.clone(), policy, framer_core::gate::ConcurrencyGate::new());
    (state, ports)
}

async fn register(state: AppState, kind: Kind, n: u128, email: &str)
    -> pool_control::CleanupOutcome<Json<Value>> {
    let body = serde_json::json!({"account_id": id(n), "email": email,
        "alias": format!("fixture-{n}"), "project_id": "fixture-project",
        "email_token": format!("token-{n}"), "session_cookie": cookie(n)});
    match kind {
        Kind::Login => complete_login_observed(State(state),
            Json(serde_json::from_value(body).unwrap())).await,
        Kind::Import => import_account_observed(State(state),
            Json(serde_json::from_value(body).unwrap())).await,
    }
}

#[async_trait::async_trait]
impl SessionStore for Ports {
    async fn save(&self, session: &Session) -> Result<(), String> {
        let n = number(session);
        assert_eq!(session.active_user.0, self.user(n));
        self.call(n, "save");
        self.hold(n, Hold::Save).await;
        if n == 1 && self.failure == Failure::Save {
            return Err("fixture session save failed".into());
        }
        if n == 2 {
            *self.second_previous_session.lock().unwrap() =
                self.saved.lock().unwrap().get(&self.user(n)).cloned();
        }
        self.saved.lock().unwrap().insert(self.user(n), session.cookie_header.clone());
        Ok(())
    }
    async fn load(&self, _: &FramerUserId) -> Result<Option<Session>, String> {
        panic!("unexpected session load");
    }
    async fn delete(&self, _: &FramerUserId) -> Result<(), String> {
        panic!("unexpected session deletion");
    }
}

#[async_trait::async_trait]
impl StateStore for Ports {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        Ok(self.records.lock().unwrap().iter().find(|a| a.id == id).cloned())
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        Ok(self.records.lock().unwrap().clone())
    }
    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String> {
        self.call(account.id.0.as_u128(), "upsert");
        self.hold(account.id.0.as_u128(), Hold::Publish).await;
        if account.id == id(1) && self.failure == Failure::Publish {
            return Err("fixture account publication failed".into());
        }
        let mut records = self.records.lock().unwrap();
        records.retain(|a| a.id != account.id);
        records.push(account.clone());
        Ok(())
    }
    async fn remove(&self, _: AccountId) -> Result<(), String> {
        panic!("unexpected account removal");
    }
}

#[async_trait::async_trait]
impl BrowserBridge for Ports {
    async fn auth_complete(&self, account: &AccountId, token: &str)
        -> Result<(String, Value), String> {
        let n = account.0.as_u128();
        assert_eq!(token, format!("token-{n}"));
        self.call(n, "auth_complete");
        if n == 2 { self.second_started.notify_one(); }
        Ok((cookie(n), self.identity(n)))
    }
    async fn register_account_proxy(&self, account: &AccountId, proxy: Option<String>)
        -> Result<(), String> {
        assert!(proxy.is_none());
        self.call(account.0.as_u128(), "proxy");
        Ok(())
    }
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        panic!("unexpected browser mint");
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("unexpected login start");
    }
}

#[async_trait::async_trait]
impl FramerClient for Ports {
    async fn create_workspace(&self, _: &Session, _: &WorkspaceSpec) -> Result<TeamId, String> {
        panic!("unexpected workspace creation");
    }
    async fn freechat_long(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>) -> Result<AssembledResponse, String> {
        panic!("unexpected Messages request");
    }
    async fn native_long(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>) -> Result<AssembledResponses, String> {
        panic!("unexpected Responses request");
    }
    async fn team_projects(&self, _: &Session, _: &TeamId) -> Result<Vec<ProjectId>, String> {
        panic!("unexpected project listing");
    }
    async fn create_project(&self, _: &Session, _: &TeamId) -> Result<ProjectId, String> {
        panic!("unexpected project creation");
    }
    async fn claim_workspace_grant(&self, session: &Session) -> Result<bool, String> {
        let n = number(session);
        assert_eq!(self.saved.lock().unwrap().get(&self.user(n)),
            Some(&session.cookie_header), "grant must follow session save");
        self.call(n, "grant");
        self.hold(n, Hold::Grant).await;
        Ok(true)
    }
    async fn request_magic_link(&self, _: &str, _: &CastleToken) -> Result<(), String> {
        panic!("unexpected magic link request");
    }
    async fn complete_magic_link(&self, _: &str, _: &CastleToken) -> Result<Session, String> {
        panic!("unexpected magic link completion");
    }
    async fn run_inference(&self, _: &Session, _: &dyn BrowserBridge,
        _: &AccountId, _: &ProjectId, _: &InferenceRequest) -> Result<InferenceResult, String> {
        panic!("unexpected inference");
    }
    async fn proxy(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: &str, _: Value, _: &str) -> Result<ProxyResponse, String> {
        panic!("unexpected proxy request");
    }
    async fn me(&self, session: &Session) -> Result<Value, String> {
        let n = number(session);
        if session.active_user.0.is_empty() {
            self.call(n, "probe");
            if n == 2 { self.second_started.notify_one(); }
        } else {
            assert_eq!(session.active_user.0, self.user(n));
        }
        Ok(self.identity(n))
    }
    async fn usage(&self, session: &Session, team: &TeamId)
        -> Result<CreditSnapshot, String> {
        let n = number(session);
        assert_eq!(team.0, self.team(n));
        self.hold(n, Hold::Usage).await;
        Ok(CreditSnapshot { available: 100.0, funds: 1, grants: 1, allocated: 100.0 })
    }
}

async fn compete(kinds: [Kind; 2], hold: Hold, failure: Failure,
    distinct: bool) -> Competition {
    let (state, ports) = fixture(kinds, hold, failure, distinct);
    let mut first = tokio::spawn(register(state.clone(), kinds[0], 1, EMAIL));
    let entered = timeout(WAIT, ports.entered.notified()).await.is_ok();
    let email = if distinct { "other@example.invalid" }
        else { "  SHARED@Example.Invalid  " };
    let mut second = tokio::spawn(register(state.clone(), kinds[1], 2, email));
    let started = timeout(WAIT, ports.second_started.notified()).await.is_ok();
    let window = if hold == Hold::Usage { WAIT } else { SHORT };
    let ready = timeout(window, &mut second).await.ok();
    let second_waited = ready.is_none();
    let midway = ports.records.lock().unwrap().clone();
    let saved_midway = ports.saved.lock().unwrap().clone();
    ports.release.notify_one();
    // Settle both tasks before assertions, including on the unfixed source.
    let first = settle(&mut first).await;
    let second = match ready {
        Some(result) => result,
        None => settle(&mut second).await,
    };
    assert!(entered, "first registration did not reach the held call");
    assert!(started, "second registration did not reach its identity check");
    assert!(state.serving_accounts.mutations.lock().unwrap().is_empty());
    Competition {
        ports, first: first.expect("first registration task failed"),
        second: second.expect("second registration task failed"),
        second_waited, midway, saved_midway,
    }
}

fn assert_success(outcome: &Outcome, kind: Kind) {
    assert!(outcome.cleanup_confirmed);
    let result = &outcome.result.as_ref().expect("registration rejected").0;
    assert_eq!(result["status"], match kind {
        Kind::Login => "signed-in", Kind::Import => "imported",
    });
    if kind == Kind::Import { assert_eq!(result["serving_allowed"], false); }
}

fn assert_duplicate(outcome: &Outcome) {
    assert!(outcome.cleanup_confirmed);
    let expected = format!("{EMAIL} is already added as account fixture-1 \u{2014} not adding it twice");
    assert!(matches!(&outcome.result, Err(ApiError::Internal(error))
        if error == &expected));
}

fn assert_only_account(ports: &Ports, n: u128) {
    let records = ports.records.lock().unwrap();
    assert_eq!(records.len(), 1, "duplicate account records were persisted");
    let account = &records[0];
    assert_eq!(account.id, id(n));
    assert_eq!(account.email.as_deref(), Some(EMAIL));
    assert_eq!(account.framer_user_id.0, ports.user(n));
    assert_eq!(account.state, framer_core::account::AccountState::Ready);
    assert_eq!(*ports.saved.lock().unwrap(), BTreeMap::from([(ports.user(n), cookie(n))]));
}

fn side_effects(ports: &Ports, n: u128) -> Vec<&'static str> {
    ports.calls.lock().unwrap().iter().filter_map(|&(who, call)| {
        (who == n && matches!(call, "save" | "grant" | "proxy" | "upsert"))
            .then_some(call)
    }).collect()
}

fn assert_save_order(ports: &Ports, n: u128, kind: Kind) {
    let calls = side_effects(ports, n);
    let prefix: &[&str] = match kind {
        Kind::Login => &["save", "grant", "upsert"],
        Kind::Import => &["save", "grant", "proxy", "upsert"],
    };
    assert!(calls.starts_with(prefix));
    assert_eq!(calls.iter().filter(|&&call| call == "save").count(), 1);
    assert_eq!(calls.iter().filter(|&&call| call == "grant").count(), 1);
}

async fn assert_serialized(kinds: [Kind; 2]) {
    for hold in [Hold::Grant, Hold::Publish] {
        assert_serialized_at(kinds, hold).await;
    }
}

async fn assert_serialized_at(kinds: [Kind; 2], hold: Hold) {
    let race = compete(kinds, hold, Failure::None, false).await;
    assert_only_account(&race.ports, 1);
    assert_success(&race.first, kinds[0]);
    assert_duplicate(&race.second);
    assert!(race.second_waited, "second caller passed pending publication");
    assert!(race.midway.is_empty());
    assert_eq!(race.saved_midway, BTreeMap::from([(race.ports.user(1), cookie(1))]));
    assert_save_order(&race.ports, 1, kinds[0]);
    assert!(side_effects(&race.ports, 2).is_empty());
}

#[tokio::test]
async fn concurrent_imports_preserve_one_normalized_email() {
    assert_serialized([Kind::Import, Kind::Import]).await;
}

#[tokio::test]
async fn concurrent_logins_preserve_one_normalized_email() {
    assert_serialized([Kind::Login, Kind::Login]).await;
}

#[tokio::test]
async fn pending_login_blocks_duplicate_import() {
    assert_serialized([Kind::Login, Kind::Import]).await;
}

#[tokio::test]
async fn pending_import_blocks_duplicate_login() {
    assert_serialized([Kind::Import, Kind::Login]).await;
}

async fn assert_failure_releases(failure: Failure) {
    let (hold, message) = match failure {
        Failure::Save => (Hold::Save, "fixture session save failed"),
        Failure::Publish => (Hold::Grant, "fixture account publication failed"),
        Failure::None => unreachable!(),
    };
    for kinds in [[Kind::Login, Kind::Import], [Kind::Import, Kind::Login]] {
        let race = compete(kinds, hold, failure, false).await;
        assert!(race.first.cleanup_confirmed);
        assert!(matches!(&race.first.result, Err(ApiError::Internal(error))
            if error == message));
        assert_success(&race.second, kinds[1]);
        assert_only_account(&race.ports, 2);
        assert_save_order(&race.ports, 2, kinds[1]);
        assert!(race.second_waited);
        assert!(race.midway.is_empty());
        let saved = if failure == Failure::Save { BTreeMap::new() }
            else { BTreeMap::from([(race.ports.user(1), cookie(1))]) };
        assert_eq!(race.saved_midway, saved);
        // Initial upsert failure still leaves the saved session and grant.
        assert_eq!(*race.ports.second_previous_session.lock().unwrap(),
            (failure == Failure::Publish).then(|| cookie(1)));
        let failed_calls: &[&str] = if failure == Failure::Save { &["save"] }
            else if kinds[0] == Kind::Login { &["save", "grant", "upsert"] }
            else { &["save", "grant", "proxy", "upsert"] };
        assert_eq!(side_effects(&race.ports, 1), failed_calls);
    }
}

#[tokio::test]
async fn failed_session_save_releases_waiting_registration() {
    assert_failure_releases(Failure::Save).await;
}

#[tokio::test]
async fn failed_initial_upsert_releases_waiting_registration() {
    assert_failure_releases(Failure::Publish).await;
}

#[tokio::test]
async fn registration_releases_before_credit_admission() {
    for kinds in [[Kind::Login, Kind::Import], [Kind::Import, Kind::Login]] {
        let race = compete(kinds, Hold::Usage, Failure::None, true).await;
        assert_success(&race.first, kinds[0]);
        assert_success(&race.second, kinds[1]);
        assert!(!race.second_waited, "credit scan held the registration guard");
        assert_eq!(race.midway.len(), 2);
        assert_eq!(*race.ports.saved.lock().unwrap(), BTreeMap::from([
            (race.ports.user(1), cookie(1)), (race.ports.user(2), cookie(2)),
        ]));
        let records = race.ports.records.lock().unwrap();
        assert_eq!(records.len(), 2);
        for n in [1, 2] {
            let account = records.iter().find(|a| a.id == id(n)).unwrap();
            assert_eq!(account.framer_user_id.0, race.ports.user(n));
            assert_eq!(account.current_team.as_ref().unwrap().0, race.ports.team(n));
            assert_save_order(&race.ports, n, kinds[n as usize - 1]);
        }
    }
}

#[tokio::test]
async fn cancelling_waiter_releases_management_without_publishing() {
    for kinds in [[Kind::Login, Kind::Import], [Kind::Import, Kind::Login]] {
        let (state, ports) = fixture(kinds, Hold::Grant, Failure::None, false);
        let mut first = tokio::spawn(register(state.clone(), kinds[0], 1, EMAIL));
        let entered = timeout(WAIT, ports.entered.notified()).await.is_ok();
        let mut waiting = tokio::spawn(register(state.clone(), kinds[1], 2, EMAIL));
        let started = timeout(WAIT, ports.second_started.notified()).await.is_ok();
        let ready = timeout(SHORT, &mut waiting).await.ok();
        let cancelled = if let Some(result) = ready {
            result.err().is_some_and(|error| error.is_cancelled())
        } else {
            waiting.abort();
            settle(&mut waiting).await.err().is_some_and(|error| error.is_cancelled())
        };
        ports.release.notify_one();
        let first = settle(&mut first).await;
        let mut retry = tokio::spawn(register(state.clone(), kinds[1], 2, EMAIL));
        let retry = settle(&mut retry).await;
        assert!(entered && started);
        assert_only_account(&ports, 1);
        assert!(cancelled, "waiter completed before it could be cancelled");
        assert_success(&first.expect("first registration task failed"), kinds[0]);
        assert_duplicate(&retry.expect("retry registration task failed"));
        assert!(side_effects(&ports, 2).is_empty());
        assert!(state.serving_accounts.mutations.lock().unwrap().is_empty());
    }
}
