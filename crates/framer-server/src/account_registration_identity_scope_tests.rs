//! Replacing a Framer login cannot publish the previous login's extra scopes.
use super::*;
use framer_core::account::WorkspaceScope;
use tokio::time::{timeout, Duration};

const COOKIE: &str = "session=fixture-only";
const PROBE_ERROR: &str = "fixture identity usage failed";
const WAIT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case { AdmissionError, SecondFailed, SecondPartial, Complete }

struct IdentityPorts {
    inner: Arc<FixturePorts>,
    case: Case,
    scans: AtomicUsize,
    calls: Mutex<Vec<&'static str>>,
    writes: Mutex<Vec<ManagedAccount>>,
}

impl IdentityPorts {
    fn record(&self, call: &'static str) { self.calls.lock().unwrap().push(call); }
    fn identity(&self) -> Value {
        json!({"id": "user-2", "spaces": [
            {"scope": "organization", "ownerId": "team-2"},
            {"scope": "organization", "ownerId": "new-extra"}
        ]})
    }
    fn check_session(&self, session: &Session) {
        assert_eq!(session.active_user.0, "user-2");
        assert_eq!(session.cookie_header, COOKIE);
    }
}

#[async_trait::async_trait]
impl BrowserBridge for IdentityPorts {
    async fn auth_complete(&self, id: &AccountId, token: &str) -> Result<(String, Value), String> {
        assert_eq!(*id, account(2).id);
        assert_eq!(token, "fixture-token");
        self.record("auth_complete");
        Ok((COOKIE.into(), self.identity()))
    }
    async fn register_account_proxy(
        &self, id: &AccountId, proxy: Option<String>,
    ) -> Result<(), String> {
        assert_eq!(*id, account(2).id);
        assert_eq!(proxy, account(2).proxy);
        self.record("proxy");
        self.inner.register_account_proxy(id, proxy).await
    }
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        self.inner.unexpected("browser");
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        self.inner.unexpected("browser");
    }
}

#[async_trait::async_trait]
impl SessionStore for IdentityPorts {
    async fn save(&self, session: &Session) -> Result<(), String> {
        self.check_session(session);
        self.record("save");
        self.inner.save(session).await
    }
    async fn load(&self, _: &FramerUserId) -> Result<Option<Session>, String> {
        self.inner.unexpected("session");
    }
    async fn delete(&self, _: &FramerUserId) -> Result<(), String> {
        self.inner.unexpected("session");
    }
}

#[async_trait::async_trait]
impl StateStore for IdentityPorts {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        self.record("get");
        self.inner.get(id).await
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        self.record("list");
        self.inner.list().await
    }
    async fn upsert(&self, current: &ManagedAccount) -> Result<(), String> {
        self.record("upsert");
        self.writes.lock().unwrap().push(current.clone());
        self.inner.upsert(current).await
    }
    async fn remove(&self, _: AccountId) -> Result<(), String> {
        self.inner.unexpected("state");
    }
}

#[async_trait::async_trait]
impl FramerClient for IdentityPorts {
    async fn me(&self, session: &Session) -> Result<Value, String> {
        assert_eq!(session.cookie_header, COOKIE);
        if session.active_user.0.is_empty() { self.record("probe"); }
        else {
            self.check_session(session);
            self.scans.fetch_add(1, Ordering::SeqCst);
            self.record("scan");
        }
        Ok(self.identity())
    }
    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        self.check_session(session);
        assert!(matches!(team.0.as_str(), "team-2" | "new-extra"));
        self.record(if team.0 == "team-2" { "primary_usage" } else { "extra_usage" });
        let scan = self.scans.load(Ordering::SeqCst);
        assert!(matches!(scan, 1 | 2));
        if self.case == Case::AdmissionError
            || (scan == 2 && (self.case == Case::SecondFailed
                || (self.case == Case::SecondPartial && team.0 == "team-2"))) {
            return Err(PROBE_ERROR.into());
        }
        Ok(credit(if team.0 == "team-2" { 100.0 } else { 20.0 }))
    }
    async fn claim_workspace_grant(&self, session: &Session) -> Result<bool, String> {
        self.check_session(session);
        assert_eq!(self.inner.saved.lock().unwrap().len(), 1);
        self.record("grant");
        Ok(false)
    }
    async fn team_projects(&self, session: &Session, team: &TeamId) -> Result<Vec<ProjectId>, String> {
        self.check_session(session);
        assert_eq!(self.case, Case::Complete);
        assert_eq!(team.0, "new-extra");
        assert_eq!(self.scans.load(Ordering::SeqCst), 2);
        self.record("projects");
        Ok(vec![ProjectId("new-extra-project".into())])
    }
    async fn create_project(&self, _: &Session, _: &TeamId) -> Result<ProjectId, String> {
        self.inner.unexpected("client");
    }
    async fn create_workspace(&self, _: &Session, _: &WorkspaceSpec) -> Result<TeamId, String> {
        self.inner.unexpected("client");
    }
    async fn request_magic_link(&self, _: &str, _: &CastleToken) -> Result<(), String> {
        self.inner.unexpected("client");
    }
    async fn complete_magic_link(&self, _: &str, _: &CastleToken) -> Result<Session, String> {
        self.inner.unexpected("client");
    }
    async fn run_inference(
        &self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: &InferenceRequest,
    ) -> Result<InferenceResult, String> {
        self.inner.unexpected("client");
    }
    async fn proxy(
        &self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: &str, _: Value, _: &str,
    ) -> Result<ProxyResponse, String> {
        self.inner.unexpected("client");
    }
    async fn freechat_long(
        &self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
    ) -> Result<AssembledResponse, String> {
        self.inner.unexpected("client");
    }
    async fn native_long(
        &self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
    ) -> Result<AssembledResponses, String> {
        self.inner.unexpected("client");
    }
}

fn credit(available: f64) -> CreditSnapshot {
    CreditSnapshot { available, funds: 1, grants: 1, allocated: available }
}

fn scope(team: &str) -> WorkspaceScope {
    WorkspaceScope { team: TeamId(team.into()), project: ProjectId(format!("{team}-project")) }
}

fn identity_fixture(login: bool, case: Case) -> (AppState, Arc<IdentityPorts>, ManagedAccount) {
    let (_, inner) = fixture(true);
    let mut initial = account(2);
    initial.framer_user_id = FramerUserId("previous-user".into());
    initial.credit = Some(credit(7.0));
    initial.consecutive_failures = 3;
    initial.cooldown_step = 2;
    initial.cooling_until_ms = Some(12_345);
    initial.extra_workspaces = vec![scope("previous-extra-a"), scope("previous-extra-b")];
    *inner.records.lock().unwrap() = vec![account(1), initial.clone()];
    let ports = Arc::new(IdentityPorts {
        inner, case, scans: AtomicUsize::new(0),
        calls: Mutex::new(Vec::new()), writes: Mutex::new(Vec::new()),
    });
    let state = AppState::fixture(
        ports.clone(), ports.clone(), ports.clone(), ports.clone(),
        policy(if login { &[2] } else { &[1] }), framer_core::gate::ConcurrencyGate::new(),
    );
    (state, ports, initial)
}

async fn register(state: AppState, login: bool) -> crate::pool_control::CleanupOutcome<Json<Value>> {
    if login {
        let body = serde_json::from_value(json!({
            "account_id": account(2).id, "email_token": "fixture-token"
        })).unwrap();
        crate::complete_login_observed(State(state), Json(body)).await
    } else {
        let body = serde_json::from_value(json!({
            "account_id": account(2).id, "session_cookie": COOKIE,
            "expected_framer_user_id": "user-2"
        })).unwrap();
        crate::import_account_observed(State(state), Json(body)).await
    }
}

async fn exercise(login: bool, case: Case) {
    let (state, ports, initial) = identity_fixture(login, case);
    let serving = state.serving_accounts.clone();
    let config = serving.config();
    let outcome = timeout(WAIT, register(state.clone(), login)).await
        .expect("identity fixture did not settle");
    assert!(outcome.cleanup_confirmed);
    assert_result(outcome.result, login, case);
    assert_saved(&ports, &initial, case);
    assert_calls(&ports, login, case);
    assert_eq!(serving.config(), config);
    assert_settled(state, &ports, login);
}

fn assert_result(result: Result<Json<Value>, ApiError>, login: bool, case: Case) {
    if case == Case::AdmissionError {
        assert!(matches!(result, Err(ApiError::Internal(ref error))
            if error == "framer: fixture identity usage failed"));
        return;
    }
    let response = result.unwrap().0;
    assert_eq!(response["status"], if login { "signed-in" } else { "imported" });
    assert_eq!(response["account_id"], json!(account(2).id));
    assert_eq!(response["framer_user_id"], "user-2");
    assert_eq!(response["team"], "team-2");
    assert_eq!(response["project"], "project-2");
    if !login { assert_eq!(response["serving_allowed"], false); }
}

fn assert_saved(ports: &IdentityPorts, initial: &ManagedAccount, case: Case) {
    let mut current = initial.clone();
    current.framer_user_id = FramerUserId("user-2".into());
    current.extra_workspaces.clear();
    let mut stages = vec![current.clone()];
    if case != Case::AdmissionError {
        current.credit = Some(credit(100.0));
        stages.push(current.clone());
        if case == Case::Complete { current.extra_workspaces = vec![scope("new-extra")]; }
        stages.push(current.clone());
    }
    assert_eq!(json!(*ports.writes.lock().unwrap()), json!(stages));
    assert_eq!(ports.inner.state_writes.load(Ordering::SeqCst), stages.len());
    assert_eq!(json!(*ports.inner.records.lock().unwrap()), json!([account(1), current]));
    let records = ports.inner.records.lock().unwrap();
    let actual = records.iter().find(|record| record.id == initial.id).unwrap();
    let mut scopes = vec![WorkspaceScope {
        team: TeamId("team-2".into()), project: ProjectId("project-2".into()),
    }];
    if case == Case::Complete { scopes.push(scope("new-extra")); }
    assert_eq!(json!(actual.all_scopes()), json!(scopes));
}

fn assert_calls(ports: &IdentityPorts, login: bool, case: Case) {
    let mut expected = vec![if login { "auth_complete" } else { "probe" },
        "get", "save", "grant"];
    if !login { expected.push("proxy"); }
    expected.extend(["upsert", "list", "scan"]);
    if case != Case::AdmissionError {
        expected.extend(["get", "list", "upsert", "scan"]);
        if case == Case::Complete { expected.push("projects"); }
        expected.extend(["get", "list", "upsert"]);
    }
    let scans = if case == Case::AdmissionError { 1 } else { 2 };
    assert_eq!(ports.scans.load(Ordering::SeqCst), scans);
    let calls = ports.calls.lock().unwrap();
    let without_probes: Vec<_> = calls.iter().copied()
        .filter(|call| !call.ends_with("_usage")).collect();
    assert_eq!(without_probes, expected);
    assert_eq!(calls.iter().filter(|&&call| call == "primary_usage").count(), scans);
    assert_eq!(calls.iter().filter(|&&call| call == "extra_usage").count(), scans);
}

fn assert_settled(state: AppState, ports: &Arc<IdentityPorts>, login: bool) {
    let saved = ports.inner.saved.lock().unwrap();
    assert_eq!(saved.len(), 1);
    ports.check_session(&saved[0]);
    drop(saved);
    let proxies = if login { Vec::new() } else { vec![(account(2).id, account(2).proxy)] };
    assert_eq!(*ports.inner.registered_proxies.lock().unwrap(), proxies);
    assert_eq!(ports.inner.session_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.inner.browser_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.inner.client_calls.load(Ordering::SeqCst), 0);
    assert!(state.serving_accounts.mutations.lock().unwrap().is_empty());
    assert!(state.serving_accounts.registration.try_lock().is_ok());
    assert!(state.serving_accounts.health_update.try_lock().is_ok());
    assert!(state.serving_accounts.residents.snapshot().unwrap().residents.iter()
        .all(|slot| slot.users == 0 && !slot.uncertain && slot.replacement.is_none()));
    drop(state);
    assert_eq!(Arc::strong_count(ports), 1, "background work retained fixture ports");
}

#[tokio::test]
async fn login_changed_identity_clears_extras_before_failed_admission() {
    exercise(true, Case::AdmissionError).await;
}

#[tokio::test]
async fn import_changed_identity_clears_extras_before_failed_admission() {
    exercise(false, Case::AdmissionError).await;
}

#[tokio::test]
async fn login_changed_identity_drops_old_extras_after_incomplete_scan() {
    for case in [Case::SecondFailed, Case::SecondPartial] { exercise(true, case).await; }
}

#[tokio::test]
async fn import_changed_identity_drops_old_extras_after_incomplete_scan() {
    for case in [Case::SecondFailed, Case::SecondPartial] { exercise(false, case).await; }
}

#[tokio::test]
async fn login_changed_identity_publishes_only_discovered_extras() {
    exercise(true, Case::Complete).await;
}

#[tokio::test]
async fn import_changed_identity_publishes_only_discovered_extras() {
    exercise(false, Case::Complete).await;
}
