//! A replacement login cannot inherit an unobserved primary workspace.
use super::*;
use framer_core::account::WorkspaceScope;
use tokio::time::{timeout, Duration};

const COOKIE: &str = "session=fixture-only";
const PROFILE_ERROR: &str = "fixture missing-team profile failed";
const USAGE_ERROR: &str = "fixture missing-team usage failed";
const WAIT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case { ProfileError, UsageError, Success }

struct MissingTeamPorts {
    inner: Arc<FixturePorts>,
    case: Case,
    unmatched: bool,
    scans: AtomicUsize,
    calls: Mutex<Vec<&'static str>>,
    writes: Mutex<Vec<ManagedAccount>>,
}

impl MissingTeamPorts {
    fn record(&self, call: &'static str) { self.calls.lock().unwrap().push(call); }
    fn metadata(&self) -> Value {
        let spaces = if self.unmatched {
            vec![json!({"scope": "organization", "ownerId": "fresh-team"})]
        } else { Vec::new() };
        json!({"id": "user-2", "spaces": spaces})
    }
    fn check_session(&self, session: &Session) {
        assert_eq!(session.active_user.0, "user-2");
        assert_eq!(session.cookie_header, COOKIE);
    }
}

#[async_trait::async_trait]
impl BrowserBridge for MissingTeamPorts {
    async fn auth_complete(&self, id: &AccountId, token: &str) -> Result<(String, Value), String> {
        assert_eq!(*id, account(2).id);
        assert_eq!(token, "fixture-token");
        self.record("auth_complete");
        Ok((COOKIE.into(), self.metadata()))
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
impl SessionStore for MissingTeamPorts {
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
impl StateStore for MissingTeamPorts {
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
impl FramerClient for MissingTeamPorts {
    async fn me(&self, session: &Session) -> Result<Value, String> {
        assert_eq!(session.cookie_header, COOKIE);
        if session.active_user.0.is_empty() {
            self.record("probe");
            return Ok(self.metadata());
        }
        self.check_session(session);
        assert!(self.scans.fetch_add(1, Ordering::SeqCst) < 2);
        self.record("scan");
        if self.case == Case::ProfileError { return Err(PROFILE_ERROR.into()); }
        Ok(json!({"id": "user-2", "spaces": [
            {"scope": "organization", "ownerId": "fresh-team"}
        ]}))
    }
    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        self.check_session(session);
        assert_eq!(team.0, "fresh-team");
        self.record("usage");
        if self.case == Case::UsageError { return Err(USAGE_ERROR.into()); }
        assert_eq!(self.case, Case::Success);
        Ok(credit(100.0))
    }
    async fn claim_workspace_grant(&self, session: &Session) -> Result<bool, String> {
        self.check_session(session);
        assert!(!self.unmatched);
        assert_eq!(self.inner.saved.lock().unwrap().len(), 1);
        self.record("grant");
        Ok(false)
    }
    async fn team_projects(&self, session: &Session, team: &TeamId) -> Result<Vec<ProjectId>, String> {
        self.check_session(session);
        assert_eq!(self.case, Case::Success);
        assert_eq!(team.0, "fresh-team");
        assert_eq!(self.scans.load(Ordering::SeqCst), 1);
        assert_eq!(self.inner.state_writes.load(Ordering::SeqCst), 2);
        self.record("projects");
        Ok(vec![ProjectId("fresh-project".into())])
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

fn scope(team: &str, project: &str) -> WorkspaceScope {
    WorkspaceScope { team: TeamId(team.into()), project: ProjectId(project.into()) }
}

fn missing_team_fixture(
    login: bool, case: Case, same: bool, unmatched: bool,
) -> (AppState, Arc<MissingTeamPorts>, ManagedAccount) {
    assert!(!unmatched || (!login && !same && case == Case::ProfileError));
    let (_, inner) = fixture(true);
    let mut initial = account(2);
    if !same { initial.framer_user_id = FramerUserId("previous-user".into()); }
    initial.credit = Some(credit(7.0));
    initial.consecutive_failures = 3;
    initial.cooldown_step = 2;
    initial.cooling_until_ms = Some(12_345);
    initial.extra_workspaces = vec![scope("saved-extra", "saved-extra-project")];
    *inner.records.lock().unwrap() = vec![account(1), initial.clone()];
    let ports = Arc::new(MissingTeamPorts {
        inner, case, unmatched, scans: AtomicUsize::new(0),
        calls: Mutex::new(Vec::new()), writes: Mutex::new(Vec::new()),
    });
    let state = AppState::fixture(
        ports.clone(), ports.clone(), ports.clone(), ports.clone(),
        policy(if login { &[2] } else { &[1] }),
        framer_core::gate::ConcurrencyGate::new(),
    );
    (state, ports, initial)
}

async fn register(
    state: AppState, login: bool, unmatched: bool,
) -> crate::pool_control::CleanupOutcome<Json<Value>> {
    if login {
        let body = serde_json::from_value(json!({
            "account_id": account(2).id, "email_token": "fixture-token"
        })).unwrap();
        crate::complete_login_observed(State(state), Json(body)).await
    } else {
        let body = serde_json::from_value(json!({
            "account_id": account(2).id, "session_cookie": COOKIE,
            "expected_framer_user_id": "user-2",
            "team_id": unmatched.then_some("unlisted-team")
        })).unwrap();
        crate::import_account_observed(State(state), Json(body)).await
    }
}

async fn exercise(login: bool, case: Case, same: bool, unmatched: bool) {
    let (state, ports, initial) = missing_team_fixture(login, case, same, unmatched);
    let serving = state.serving_accounts.clone();
    let config = serving.config();
    let outcome = timeout(WAIT, register(state.clone(), login, unmatched)).await
        .expect("missing-team fixture did not settle");
    assert!(outcome.cleanup_confirmed);
    assert_result(outcome.result, login, case);
    assert_saved(&ports, &initial, same);
    assert_calls(&ports, login);
    assert_eq!(serving.config(), config);
    assert_settled(state, &ports, login);
}

fn assert_result(result: Result<Json<Value>, ApiError>, login: bool, case: Case) {
    if case != Case::Success {
        let error = if case == Case::ProfileError { PROFILE_ERROR } else { USAGE_ERROR };
        let expected = format!("framer: {error}");
        assert!(matches!(result, Err(ApiError::Internal(ref message)) if message == &expected));
        return;
    }
    let response = result.unwrap().0;
    assert_eq!(response["status"], if login { "signed-in" } else { "imported" });
    assert_eq!(response["account_id"], json!(account(2).id));
    assert_eq!(response["framer_user_id"], "user-2");
    assert_eq!(response["team"], "fresh-team");
    assert_eq!(response["project"], "fresh-project");
    if !login { assert_eq!(response["serving_allowed"], false); }
}

fn assert_saved(ports: &MissingTeamPorts, initial: &ManagedAccount, same: bool) {
    let mut current = initial.clone();
    if !same {
        current.framer_user_id = FramerUserId("user-2".into());
        current.current_team = None;
        current.current_project = None;
        current.extra_workspaces.clear();
    }
    let mut stages = vec![current.clone()];
    if ports.case == Case::Success {
        current.current_team = Some(TeamId("fresh-team".into()));
        current.current_project = None;
        current.credit = Some(credit(100.0));
        stages.push(current.clone());
        current.current_project = Some(ProjectId("fresh-project".into()));
        current.extra_workspaces.clear();
        stages.push(current.clone());
    }
    let writes = ports.writes.lock().unwrap();
    assert_eq!(json!(*writes), json!(stages));
    assert_eq!(ports.inner.state_writes.load(Ordering::SeqCst), stages.len());
    assert_eq!(json!(*ports.inner.records.lock().unwrap()), json!([account(1), current]));
    assert_scopes(&writes, same, ports.case);
}

fn assert_scopes(writes: &[ManagedAccount], same: bool, case: Case) {
    let expected = if case == Case::Success {
        vec![(false, vec![]), (false, vec![]),
            (true, vec![scope("fresh-team", "fresh-project")])]
    } else if same {
        vec![(true, vec![scope("team-2", "project-2"),
            scope("saved-extra", "saved-extra-project")])]
    } else {
        vec![(false, vec![])]
    };
    assert_eq!(writes.len(), expected.len());
    for (saved, (eligible, scopes)) in writes.iter().zip(expected) {
        assert_eq!(saved.is_eligible(), eligible);
        assert_eq!(json!(saved.all_scopes()), json!(scopes));
    }
}

fn assert_calls(ports: &MissingTeamPorts, login: bool) {
    let mut expected = vec![if login { "auth_complete" } else { "probe" },
        "get", "list", "save"];
    if !ports.unmatched { expected.push("grant"); }
    if !login { expected.push("proxy"); }
    expected.extend(["upsert", "list", "scan"]);
    if ports.case != Case::ProfileError { expected.push("usage"); }
    if ports.case == Case::Success {
        expected.extend(["get", "list", "upsert", "projects", "scan", "usage",
            "get", "list", "upsert"]);
    }
    assert_eq!(*ports.calls.lock().unwrap(), expected);
    assert_eq!(ports.scans.load(Ordering::SeqCst),
        if ports.case == Case::Success { 2 } else { 1 });
}

fn assert_settled(state: AppState, ports: &Arc<MissingTeamPorts>, login: bool) {
    let saved = ports.inner.saved.lock().unwrap();
    assert_eq!(saved.len(), 1);
    ports.check_session(&saved[0]);
    drop(saved);
    let proxies = if login { vec![] } else { vec![(account(2).id, account(2).proxy)] };
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
async fn login_changed_identity_without_team_clears_primary_before_admission_error() {
    for case in [Case::ProfileError, Case::UsageError] {
        exercise(true, case, false, false).await;
    }
}

#[tokio::test]
async fn import_changed_identity_without_team_clears_primary_before_admission_error() {
    for case in [Case::ProfileError, Case::UsageError] {
        exercise(false, case, false, false).await;
    }
    exercise(false, Case::ProfileError, false, true).await;
}

#[tokio::test]
async fn login_changed_identity_without_team_admits_fresh_scope() {
    exercise(true, Case::Success, false, false).await;
}

#[tokio::test]
async fn import_changed_identity_without_team_admits_fresh_scope() {
    exercise(false, Case::Success, false, false).await;
}

#[tokio::test]
async fn login_same_identity_without_team_preserves_saved_scope() {
    exercise(true, Case::ProfileError, true, false).await;
}

#[tokio::test]
async fn import_same_identity_without_team_preserves_saved_scope() {
    exercise(false, Case::ProfileError, true, false).await;
}
