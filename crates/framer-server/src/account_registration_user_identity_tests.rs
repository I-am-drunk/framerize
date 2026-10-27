//! Inert registration fixtures. No browser, network, filesystem or process I/O.
use super::*;
use crate::{complete_login_observed, import_account_observed, pool_control};
use framer_core::account::AccountState;
use std::collections::BTreeMap;
use std::time::Duration;

const COOKIE: &str = "session=identity-replacement-fixture";
const LIST_ERROR: &str = "identity fixture: account list failed";
const DUPLICATE: &str = "Framer user is already registered to another account";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case { Duplicate, Reauthenticate, Distinct, ListError }

struct IdentityPorts {
    target: AccountId,
    user: u128,
    case: Case,
    records: Mutex<Vec<ManagedAccount>>,
    saved: Mutex<BTreeMap<String, String>>,
    calls: Mutex<Vec<&'static str>>,
    writes: Mutex<Vec<ManagedAccount>>,
}

fn user_id(n: u128) -> FramerUserId {
    FramerUserId(format!("fixture-user-shared-prefix-{n}"))
}

fn record(n: u128) -> ManagedAccount {
    ManagedAccount {
        id: AccountId(uuid::Uuid::from_u128(n)),
        alias: format!("fixture-{n}"),
        email: Some(format!("fixture-{n}@example.invalid")),
        framer_user_id: user_id(n),
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

fn credit() -> CreditSnapshot {
    CreditSnapshot { available: 100.0, funds: 1, grants: 1, allocated: 100.0 }
}

fn fixture(case: Case, login: bool) -> (AppState, Arc<IdentityPorts>) {
    let target = if case == Case::Reauthenticate { 2 } else { 3 };
    let user = if matches!(case, Case::Duplicate | Case::Reauthenticate) { 2 } else { 3 };
    let ports = Arc::new(IdentityPorts {
        target: record(target).id, user, case,
        records: Mutex::new(vec![record(1), record(2)]),
        saved: Mutex::new(BTreeMap::from([
            (user_id(1).0, "session=prior-fixture-1".into()),
            (user_id(2).0, "session=prior-fixture-2".into()),
        ])),
        calls: Mutex::new(Vec::new()),
        writes: Mutex::new(Vec::new()),
    });
    // Imports are excluded from optional prewarm; login still has admission.
    let state = AppState::fixture(
        ports.clone(), ports.clone(), ports.clone(), ports.clone(),
        policy(&[if login { target } else { 1 }]),
        framer_core::gate::ConcurrencyGate::new(),
    );
    (state, ports)
}

impl IdentityPorts {
    fn call(&self, name: &'static str) { self.calls.lock().unwrap().push(name); }
    fn records(&self) -> Value {
        serde_json::to_value(&*self.records.lock().unwrap()).unwrap()
    }
    fn identity(&self) -> Value {
        json!({"id": user_id(self.user).0, "spaces": [
            {"scope": "organization", "ownerId": format!("team-{}", self.user)}
        ]})
    }
    fn assert_session(&self, session: &Session) {
        assert_eq!(session.active_user, user_id(self.user));
        assert_eq!(session.cookie_header, COOKIE);
    }
    fn assert_saved(&self) {
        assert_eq!(self.saved.lock().unwrap().get(&user_id(self.user).0)
            .map(String::as_str), Some(COOKIE), "session must be saved first");
    }
}

async fn invoke(state: &AppState, ports: &IdentityPorts, login: bool,
    email: Option<&str>) -> pool_control::CleanupOutcome<Json<Value>> {
    let operation = async {
        if login {
            let body = serde_json::from_value(json!({
                "account_id": ports.target, "email_token": "fixture-token", "email": email
            })).unwrap();
            complete_login_observed(State(state.clone()), Json(body)).await
        } else {
            let body = serde_json::from_value(json!({
                "account_id": ports.target, "session_cookie": COOKIE, "email": email
            })).unwrap();
            import_account_observed(State(state.clone()), Json(body)).await
        }
    };
    tokio::time::timeout(Duration::from_secs(3), operation).await
        .expect("inert registration did not settle")
}

fn assert_settled(state: AppState, ports: &Arc<IdentityPorts>) {
    assert!(state.serving_accounts.mutations.lock().unwrap().is_empty());
    assert!(state.serving_accounts.registration.try_lock().is_ok());
    assert!(state.serving_accounts.health_update.try_lock().is_ok());
    assert!(state.serving_accounts.residents.snapshot().unwrap().residents.iter()
        .all(|slot| slot.users == 0 && !slot.uncertain && slot.replacement.is_none()));
    drop(state);
    assert_eq!(Arc::strong_count(ports), 1, "background work retained fixture ports");
}

async fn rejection(login: bool, case: Case, email: Option<&str>) {
    let (state, ports) = fixture(case, login);
    let before = ports.records();
    let saved = ports.saved.lock().unwrap().clone();
    let outcome = invoke(&state, &ports, login, email).await;
    // Baselines must expose persistence effects before checking error text.
    assert_eq!(*ports.saved.lock().unwrap(), saved);
    assert_eq!(ports.records(), before);
    assert!(ports.writes.lock().unwrap().is_empty());
    assert_eq!(*ports.calls.lock().unwrap(),
        [if login { "auth_complete" } else { "me" }, "get", "list"]);
    let expected = if case == Case::ListError { LIST_ERROR } else { DUPLICATE };
    assert!(matches!(outcome.result, Err(ApiError::Internal(ref e)) if e == expected));
    assert!(outcome.cleanup_confirmed);
    assert_settled(state, &ports);
}

fn expected_registration(ports: &IdentityPorts, login: bool) -> ManagedAccount {
    let mut expected = record(ports.user);
    if ports.case == Case::Distinct {
        let device = ports.writes.lock().unwrap()[0].castle_device_id.clone();
        uuid::Uuid::parse_str(&device).expect("new registration must have a device ID");
        expected.castle_device_id = device;
        expected.alias = if login { "account" } else { "imported" }.into();
        expected.email = None;
        expected.proxy = None;
        expected.consecutive_failures = 0;
        expected.cooldown_step = 0;
    } else {
        expected.alias = "fixture-2@example.invalid".into();
    }
    expected.credit = Some(credit());
    expected
}

async fn success(login: bool, case: Case) {
    let (state, ports) = fixture(case, login);
    let mut saved = ports.saved.lock().unwrap().clone();
    let email = (case == Case::Reauthenticate).then_some("  FIXTURE-2@Example.Invalid  ");
    let outcome = invoke(&state, &ports, login, email).await;
    assert!(outcome.cleanup_confirmed);
    let response = outcome.result.unwrap().0;
    assert_eq!(response["status"], if login { "signed-in" } else { "imported" });
    assert_eq!(response["framer_user_id"], user_id(ports.user).0);
    if !login { assert_eq!(response["serving_allowed"], false); }
    let expected = expected_registration(&ports, login);
    let mut records = vec![record(1), record(2)];
    records.retain(|a| a.id != ports.target);
    records.push(expected.clone());
    assert_eq!(ports.records(), serde_json::to_value(records).unwrap());
    saved.insert(user_id(ports.user).0, COOKIE.into());
    assert_eq!(*ports.saved.lock().unwrap(), saved);
    let mut initial = expected.clone();
    initial.credit = None;
    if case == Case::Distinct { initial.current_project = None; }
    let mut admitted = initial.clone();
    admitted.credit = Some(credit());
    assert_eq!(serde_json::to_value(&*ports.writes.lock().unwrap()).unwrap(),
        serde_json::to_value([initial, admitted, expected]).unwrap());
    let calls = ports.calls.lock().unwrap().clone();
    let effects: Vec<_> = calls.iter().copied()
        .filter(|call| !matches!(*call, "get" | "list")).collect();
    let prefix: &[&str] = if login { &["auth_complete", "save", "grant", "upsert"] }
        else { &["me", "save", "grant", "proxy", "upsert"] };
    assert!(effects.starts_with(prefix));
    assert_eq!(calls.iter().filter(|&&call| call == "save").count(), 1);
    assert_eq!(calls.iter().filter(|&&call| call == "grant").count(), 1);
    assert_settled(state, &ports);
}

#[tokio::test(flavor = "current_thread")]
async fn login_duplicate_user_preserves_session_and_state() {
    for email in [None, Some("  OTHER@Example.Invalid  ")] {
        rejection(true, Case::Duplicate, email).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn import_duplicate_user_preserves_session_and_state() {
    for email in [None, Some("  OTHER@Example.Invalid  ")] {
        rejection(false, Case::Duplicate, email).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn login_same_account_reauthentication_is_allowed() {
    success(true, Case::Reauthenticate).await;
}

#[tokio::test(flavor = "current_thread")]
async fn import_same_account_reauthentication_is_allowed() {
    success(false, Case::Reauthenticate).await;
}

#[tokio::test(flavor = "current_thread")]
async fn login_distinct_user_without_email_is_allowed() {
    success(true, Case::Distinct).await;
}

#[tokio::test(flavor = "current_thread")]
async fn import_distinct_user_without_email_is_allowed() {
    success(false, Case::Distinct).await;
}

#[tokio::test(flavor = "current_thread")]
async fn login_identity_list_error_prevents_persistence() {
    rejection(true, Case::ListError, None).await;
}

#[tokio::test(flavor = "current_thread")]
async fn import_identity_list_error_prevents_persistence() {
    rejection(false, Case::ListError, None).await;
}

#[async_trait::async_trait]
impl BrowserBridge for IdentityPorts {
    async fn auth_complete(&self, account: &AccountId, token: &str)
        -> Result<(String, Value), String> {
        assert_eq!(*account, self.target);
        assert_eq!(token, "fixture-token");
        self.call("auth_complete");
        Ok((COOKIE.into(), self.identity()))
    }
    async fn register_account_proxy(&self, account: &AccountId, proxy: Option<String>)
        -> Result<(), String> {
        assert_eq!(*account, self.target);
        let expected = if self.case == Case::Reauthenticate { record(2).proxy } else { None };
        assert_eq!(proxy, expected);
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
impl SessionStore for IdentityPorts {
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
impl StateStore for IdentityPorts {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        self.call("get");
        Ok(self.records.lock().unwrap().iter().find(|a| a.id == id).cloned())
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        self.call("list");
        if self.case == Case::ListError { return Err(LIST_ERROR.into()); }
        Ok(self.records.lock().unwrap().clone())
    }
    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String> {
        self.assert_saved();
        self.call("upsert");
        self.writes.lock().unwrap().push(account.clone());
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
impl FramerClient for IdentityPorts {
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
    async fn team_projects(&self, session: &Session, team: &TeamId)
        -> Result<Vec<ProjectId>, String> {
        self.assert_session(session);
        assert_eq!(team.0, format!("team-{}", self.user));
        self.call("projects");
        Ok(vec![ProjectId(format!("project-{}", self.user))])
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
        assert_eq!(team.0, format!("team-{}", self.user));
        self.call("usage");
        Ok(credit())
    }
    async fn claim_workspace_grant(&self, session: &Session) -> Result<bool, String> {
        self.assert_session(session);
        self.assert_saved();
        self.call("grant");
        Ok(true)
    }
}
