//! In-memory handler fixtures; no browser, filesystem or upstream execution.
use super::*;
use framer_core::account::AccountState;
use framer_core::convert::{AssembledResponse, AssembledResponses, ChainEvent};
use framer_core::ids::{FramerUserId, ProjectId, TeamId};
use framer_core::pipeline::InferenceResult;
use framer_core::port::{CastleToken, ChainConfig, ProxyResponse, Session, WorkspaceSpec};
use framer_core::workspace::CreditSnapshot;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Mutex;

const COOKIE: &str = "session=replacement-fixture";

fn assert_duplicate(outcome: pool_control::CleanupOutcome<Json<Value>>) {
    assert!(outcome.cleanup_confirmed);
    assert!(matches!(outcome.result, Err(ApiError::Internal(ref error))
        if error == "fixture-2@example.invalid is already added as account fixture-2 \u{2014} not adding it twice"));
}

fn assert_unchanged(ports: &RegistrationPorts, records: &Value,
    saved: &BTreeMap<String, String>, call: &'static str) {
    assert_eq!(&ports.records(), records);
    assert_eq!(&*ports.saved.lock().unwrap(), saved);
    assert_eq!(&*ports.calls.lock().unwrap(), &[call]);
}

struct RegistrationPorts {
    target: AccountId,
    records: Mutex<Vec<ManagedAccount>>,
    saved: Mutex<BTreeMap<String, String>>,
    calls: Mutex<Vec<&'static str>>,
}

#[tokio::test]
async fn duplicate_login_preserves_sessions_state_and_grants() {
    for target in [1, 3] {
        let (state, ports) = fixture(target, true);
        let before = ports.records();
        let saved = ports.saved.lock().unwrap().clone();
        let body = serde_json::from_value(json!({
            "account_id": ports.target, "email_token": "fixture-token",
            "email": "  FIXTURE-2@Example.Invalid  "
        })).unwrap();
        let outcome = complete_login_observed(State(state), Json(body)).await;
        assert_duplicate(outcome);
        assert_unchanged(&ports, &before, &saved, "auth_complete");
    }
}

#[tokio::test]
async fn duplicate_import_preserves_sessions_state_and_grants() {
    for target in [1, 3] {
        let (state, ports) = fixture(target, false);
        let before = ports.records();
        let saved = ports.saved.lock().unwrap().clone();
        let body = serde_json::from_value(json!({
            "account_id": ports.target, "session_cookie": COOKIE,
            "email": "  FIXTURE-2@Example.Invalid  "
        })).unwrap();
        let outcome = import_account_observed(State(state), Json(body)).await;
        assert_duplicate(outcome);
        assert_unchanged(&ports, &before, &saved, "me");
    }
}

#[tokio::test]
async fn same_account_login_saves_before_grant_and_admission() {
    let (state, ports) = fixture(2, true);
    let body = serde_json::from_value(json!({
        "account_id": ports.target, "email_token": "fixture-token",
        "email": "  FIXTURE-2@Example.Invalid  "
    })).unwrap();
    let outcome = complete_login_observed(State(state), Json(body)).await;
    assert!(outcome.cleanup_confirmed);
    assert_eq!(outcome.result.unwrap().0["status"], "signed-in");
    assert_registered(&ports);
    let calls = ports.calls.lock().unwrap();
    assert!(calls.starts_with(&["auth_complete", "save", "grant", "upsert"]));
    assert_eq!(calls.iter().filter(|&&call| call == "save").count(), 1);
    assert_eq!(calls.iter().filter(|&&call| call == "grant").count(), 1);
}

#[tokio::test]
async fn same_account_import_preserves_grant_policy_and_admission() {
    for pinned in [false, true] {
        let (state, ports) = fixture(2, false);
        assert!(!state.serving_accounts.allows(&ports.target));
        let body = serde_json::from_value(json!({
            "account_id": ports.target, "session_cookie": COOKIE,
            "email": "  FIXTURE-2@Example.Invalid  ",
            "team_id": pinned.then_some("team-2")
        })).unwrap();
        let outcome = import_account_observed(State(state), Json(body)).await;
        assert!(outcome.cleanup_confirmed);
        let response = outcome.result.unwrap().0;
        assert_eq!(response["status"], "imported");
        assert_eq!(response["serving_allowed"], false);
        assert_registered(&ports);
        let calls = ports.calls.lock().unwrap();
        let prefix: &[&str] = if pinned { &["me", "save", "proxy", "upsert"] }
            else { &["me", "save", "grant", "proxy", "upsert"] };
        assert!(calls.starts_with(prefix));
        assert_eq!(calls.iter().filter(|&&call| call == "save").count(), 1);
        assert_eq!(calls.iter().filter(|&&call| call == "grant").count(),
            usize::from(!pinned));
    }
}

impl RegistrationPorts {
    fn call(&self, name: &'static str) { self.calls.lock().unwrap().push(name); }
    fn records(&self) -> Value {
        serde_json::to_value(&*self.records.lock().unwrap()).unwrap()
    }
    fn identity(&self) -> Value {
        json!({"id": "user-2", "spaces": [
            {"scope": "organization", "ownerId": "team-2"}
        ]})
    }
    fn assert_session(&self, session: &Session) {
        assert_eq!(session.active_user.0, "user-2");
        assert_eq!(session.cookie_header, COOKIE);
    }
}

fn assert_registered(ports: &RegistrationPorts) {
    let mut updated = account(2);
    updated.alias = "fixture-2@example.invalid".into();
    updated.credit = Some(CreditSnapshot {
        available: 100.0, funds: 1, grants: 1, allocated: 100.0,
    });
    assert_eq!(ports.records(), serde_json::to_value([account(1), updated]).unwrap());
    assert_eq!(*ports.saved.lock().unwrap(), BTreeMap::from([
        ("user-1".into(), "session=prior-fixture-1".into()),
        ("user-2".into(), COOKIE.into()),
    ]));
}

fn account(n: u128) -> ManagedAccount {
    ManagedAccount {
        id: AccountId(uuid::Uuid::from_u128(n)),
        alias: format!("fixture-{n}"),
        email: Some(format!("fixture-{n}@example.invalid")),
        framer_user_id: FramerUserId(format!("user-{n}")),
        castle_device_id: format!("device-{n}"),
        proxy: Some(format!("http://proxy-{n}.invalid:8080")),
        current_team: Some(TeamId(format!("team-{n}"))),
        current_project: Some(ProjectId(format!("project-{n}"))),
        extra_workspaces: Vec::new(),
        state: AccountState::Ready,
        credit: None,
        consecutive_failures: 2,
        cooldown_step: 1,
        cooling_until_ms: None,
    }
}

fn fixture(target: u128, login: bool) -> (AppState, Arc<RegistrationPorts>) {
    let ports = Arc::new(RegistrationPorts {
        target: account(target).id,
        records: Mutex::new(vec![account(1), account(2)]),
        saved: Mutex::new(BTreeMap::from([
            ("user-1".into(), "session=prior-fixture-1".into()),
            ("user-2".into(), "session=prior-fixture-2".into()),
        ])),
        calls: Mutex::new(Vec::new()),
    });
    // Exclude imports from prewarm, while login needs browser admission.
    let allowed = account(if login { target } else { 1 }).id.0.to_string();
    let state = AppState::fixture(
        ports.clone(), ports.clone(), ports.clone(), ports.clone(),
        ServingAccountPolicy::parse(Some(&allowed)).unwrap(),
        framer_core::gate::ConcurrencyGate::new(),
    );
    (state, ports)
}

#[async_trait::async_trait]
impl BrowserBridge for RegistrationPorts {
    async fn auth_complete(&self, account: &AccountId, token: &str)
        -> Result<(String, Value), String> {
        assert_eq!(*account, self.target);
        assert_eq!(token, "fixture-token");
        self.call("auth_complete");
        Ok((COOKIE.into(), self.identity()))
    }
    async fn register_account_proxy(&self, id: &AccountId, proxy: Option<String>)
        -> Result<(), String> {
        assert_eq!(*id, self.target);
        assert_eq!(proxy, account(2).proxy);
        self.call("proxy");
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
impl SessionStore for RegistrationPorts {
    async fn save(&self, session: &Session) -> Result<(), String> {
        self.assert_session(session);
        self.call("save");
        self.saved.lock().unwrap()
            .insert(session.active_user.0.clone(), session.cookie_header.clone());
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
impl StateStore for RegistrationPorts {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        Ok(self.records.lock().unwrap().iter().find(|a| a.id == id).cloned())
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        Ok(self.records.lock().unwrap().clone())
    }
    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String> {
        self.call("upsert");
        let mut records = self.records.lock().unwrap();
        records.retain(|stored| stored.id != account.id);
        records.push(account.clone());
        Ok(())
    }
    async fn remove(&self, _: AccountId) -> Result<(), String> {
        panic!("unexpected account deletion");
    }
}

#[async_trait::async_trait]
impl FramerClient for RegistrationPorts {
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
    async fn me(&self, session: &Session) -> Result<Value, String> {
        assert_eq!(session.cookie_header, COOKIE);
        self.call("me");
        Ok(self.identity())
    }
    async fn usage(&self, session: &Session, team: &TeamId)
        -> Result<CreditSnapshot, String> {
        self.assert_session(session);
        assert_eq!(team.0, "team-2");
        self.call("usage");
        Ok(CreditSnapshot { available: 100.0, funds: 1, grants: 1, allocated: 100.0 })
    }
    async fn claim_workspace_grant(&self, session: &Session) -> Result<bool, String> {
        self.assert_session(session);
        assert_eq!(self.saved.lock().unwrap().get("user-2").map(String::as_str),
            Some(COOKIE), "grant must follow session save");
        self.call("grant");
        Ok(true)
    }
}
