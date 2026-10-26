//! Concurrent registration through inert ports with manually released probes.
use super::*;
use framer_core::account::AccountState;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(3);
const PROBE_ERROR: &str = "fixture unresolved workspace probe";
type Outcome = crate::pool_control::CleanupOutcome<Json<Value>>;
type Task = tokio::task::JoinHandle<Outcome>;

#[derive(Clone, Copy, PartialEq)]
enum Kind { Login, Import }
#[derive(Clone, Copy, PartialEq)]
enum Mode { Shared, Backup, Disjoint, Partial }

struct ClaimPorts {
    inner: Arc<FixturePorts>,
    initial: Vec<ManagedAccount>,
    mode: Mode,
    held: [AtomicBool; 2],
    entered: [Notify; 2],
    release: [Notify; 2],
    calls: Mutex<Vec<(u128, &'static str)>>,
    projects: Mutex<Vec<(u128, String)>>,
}

fn cookie(n: u128) -> String { format!("session=claim-fixture-{n}") }
fn number(session: &Session) -> u128 {
    let n = session.cookie_header.strip_prefix("session=claim-fixture-")
        .unwrap().parse().unwrap();
    if !session.active_user.0.is_empty() {
        assert_eq!(session.active_user.0, format!("user-{n}"));
    }
    n
}

impl ClaimPorts {
    fn call(&self, n: u128, name: &'static str) {
        self.calls.lock().unwrap().push((n, name));
    }
    fn target(&self, n: u128) -> String {
        if self.mode == Mode::Disjoint { format!("credited-{n}") } else { "shared".into() }
    }
    fn identity(&self, n: u128) -> Value {
        let mut spaces = vec![
            json!({"scope": "organization", "ownerId": format!("metadata-{n}")}),
            json!({"scope": "organization", "ownerId": self.target(n)}),
        ];
        if n == 2 && self.mode == Mode::Partial {
            spaces.push(json!({"scope": "organization", "ownerId": "failed"}));
        }
        json!({"id": format!("user-{n}"), "spaces": spaces})
    }
}

fn claims_fixture(kinds: [Kind; 2], mode: Mode) -> (AppState, Arc<ClaimPorts>) {
    let (_, inner) = fixture(true);
    let initial: Vec<_> = (1..=2).map(|n| {
        let mut value = account(n);
        value.current_team = Some(TeamId(format!("metadata-{n}")));
        value.current_project = None;
        value.credit = Some(CreditSnapshot { available: 7.0, funds: 1, grants: 1, allocated: 7.0 });
        value.consecutive_failures = 3;
        value.cooldown_step = 2;
        value.cooling_until_ms = Some(12_345);
        value
    }).collect();
    *inner.records.lock().unwrap() = initial.clone();
    let ports = Arc::new(ClaimPorts {
        inner, initial, mode,
        held: std::array::from_fn(|_| AtomicBool::new(false)),
        entered: std::array::from_fn(|_| Notify::new()),
        release: std::array::from_fn(|_| Notify::new()),
        calls: Mutex::new(Vec::new()), projects: Mutex::new(Vec::new()),
    });
    let mut allowed: Vec<_> = kinds.iter().enumerate().filter_map(|(n, kind)|
        (*kind == Kind::Login).then_some(n as u128 + 1)).collect();
    if allowed.is_empty() { allowed.push(99); }
    let state = AppState::fixture(ports.clone(), ports.clone(), ports.clone(),
        ports.clone(), policy(&allowed), framer_core::gate::ConcurrencyGate::new());
    (state, ports)
}

#[async_trait::async_trait]
impl StateStore for ClaimPorts {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        self.inner.get(id).await
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> { self.inner.list().await }
    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String> {
        self.call(account.id.0.as_u128(), "upsert");
        self.inner.upsert(account).await
    }
    async fn remove(&self, _: AccountId) -> Result<(), String> { panic!("unexpected remove") }
}

#[async_trait::async_trait]
impl SessionStore for ClaimPorts {
    async fn save(&self, session: &Session) -> Result<(), String> {
        self.call(number(session), "save");
        self.inner.save(session).await
    }
    async fn load(&self, _: &FramerUserId) -> Result<Option<Session>, String> {
        panic!("unexpected session load")
    }
    async fn delete(&self, _: &FramerUserId) -> Result<(), String> { panic!("unexpected delete") }
}

#[async_trait::async_trait]
impl BrowserBridge for ClaimPorts {
    async fn auth_complete(&self, account: &AccountId, token: &str) -> Result<(String, Value), String> {
        let n = account.0.as_u128();
        assert_eq!(token, format!("token-{n}"));
        self.call(n, "auth_complete");
        Ok((cookie(n), self.identity(n)))
    }
    async fn register_account_proxy(&self, account: &AccountId, proxy: Option<String>)
        -> Result<(), String> {
        self.call(account.0.as_u128(), "proxy");
        self.inner.register_account_proxy(account, proxy).await
    }
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> { panic!("unexpected mint") }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> { panic!("unexpected login") }
}

#[async_trait::async_trait]
impl FramerClient for ClaimPorts {
    async fn me(&self, session: &Session) -> Result<Value, String> {
        let n = number(session);
        self.call(n, if session.active_user.0.is_empty() { "probe" } else { "me" });
        Ok(self.identity(n))
    }
    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        let n = number(session);
        self.call(n, "usage");
        if team.0 == self.target(n) && !self.held[n as usize - 1].swap(true, Ordering::SeqCst) {
            self.entered[n as usize - 1].notify_one();
            self.release[n as usize - 1].notified().await;
        }
        if n == 2 && self.mode == Mode::Partial && team.0 == "failed" {
            return Err(PROBE_ERROR.into());
        }
        let available = if team.0 == self.target(n) { 100.0 }
            else if n == 2 && self.mode == Mode::Backup { 50.0 } else { 0.0 };
        Ok(CreditSnapshot { available, funds: 1, grants: 1, allocated: available })
    }
    async fn claim_workspace_grant(&self, session: &Session) -> Result<bool, String> {
        let n = number(session);
        assert!(self.inner.saved.lock().unwrap().iter().any(|saved|
            saved.active_user == session.active_user && saved.cookie_header == session.cookie_header));
        self.call(n, "grant");
        Ok(true)
    }
    async fn team_projects(&self, session: &Session, team: &TeamId) -> Result<Vec<ProjectId>, String> {
        let n = number(session);
        self.call(n, "projects");
        self.projects.lock().unwrap().push((n, team.0.clone()));
        Ok(vec![ProjectId(format!("project-{}-{n}", team.0))])
    }
    async fn create_project(&self, _: &Session, _: &TeamId) -> Result<ProjectId, String> {
        panic!("fixture already has an existing project")
    }
    async fn request_magic_link(&self, _: &str, _: &CastleToken) -> Result<(), String> {
        panic!("unexpected magic link")
    }
    async fn complete_magic_link(&self, _: &str, _: &CastleToken) -> Result<Session, String> {
        panic!("unexpected magic link")
    }
    async fn create_workspace(&self, _: &Session, _: &WorkspaceSpec) -> Result<TeamId, String> {
        panic!("unexpected workspace creation")
    }
    async fn run_inference(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: &InferenceRequest) -> Result<InferenceResult, String> {
        panic!("unexpected inference")
    }
    async fn proxy(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: &str, _: Value, _: &str) -> Result<ProxyResponse, String> {
        panic!("unexpected proxy")
    }
    async fn freechat_long(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>) -> Result<AssembledResponse, String> {
        panic!("unexpected chain")
    }
    async fn native_long(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>) -> Result<AssembledResponses, String> {
        panic!("unexpected chain")
    }
}

async fn register(state: AppState, kind: Kind, n: u128) -> Outcome {
    let body = json!({"account_id": account(n).id,
        "alias": format!("fixture-{n}"), "email": format!("fixture-{n}@example.invalid"),
        "email_token": format!("token-{n}"), "session_cookie": cookie(n),
        "expected_framer_user_id": format!("user-{n}")});
    match kind {
        Kind::Login => crate::complete_login_observed(State(state),
            Json(serde_json::from_value(body).unwrap())).await,
        Kind::Import => crate::import_account_observed(State(state),
            Json(serde_json::from_value(body).unwrap())).await,
    }
}

async fn settle(task: &mut Task) -> Result<Outcome, tokio::task::JoinError> {
    match timeout(WAIT, &mut *task).await {
        Ok(result) => result,
        Err(_) => { task.abort(); task.await }
    }
}

struct Race {
    ports: Arc<ClaimPorts>,
    first: Outcome,
    second: Outcome,
    after_first: ManagedAccount,
    disjoint_progress: bool,
}

async fn compete(kinds: [Kind; 2], mode: Mode) -> Race {
    let (state, ports) = claims_fixture(kinds, mode);
    let serving = state.serving_accounts.clone();
    let mut first = tokio::spawn(register(state.clone(), kinds[0], 1));
    let first_entered = timeout(WAIT, ports.entered[0].notified()).await.is_ok();
    let mut second = tokio::spawn(register(state.clone(), kinds[1], 2));
    let second_entered = timeout(WAIT, ports.entered[1].notified()).await.is_ok();
    let (first, second, after_first, disjoint_progress) = if mode == Mode::Disjoint {
        ports.release[1].notify_one();
        let second = settle(&mut second).await;
        let progressed = second.is_ok() && !first.is_finished();
        ports.release[0].notify_one();
        let first = settle(&mut first).await;
        let snapshot = ports.inner.records.lock().unwrap()[0].clone();
        (first, second, snapshot, progressed)
    } else {
        ports.release[0].notify_one();
        let first = settle(&mut first).await;
        let snapshot = ports.inner.records.lock().unwrap()[0].clone();
        ports.release[1].notify_one();
        let second = settle(&mut second).await;
        (first, second, snapshot, false)
    };
    assert!(first_entered && second_entered, "both scans must start before publication");
    let mut expected_first = ports.initial[0].clone();
    expected_first.current_team = Some(TeamId(ports.target(1)));
    expected_first.current_project = Some(ProjectId(format!("project-{}-1", ports.target(1))));
    expected_first.credit = Some(CreditSnapshot {
        available: 100.0, funds: 1, grants: 1, allocated: 100.0,
    });
    assert_eq!(json!(after_first), json!(expected_first));
    assert!(serving.mutations.lock().unwrap().is_empty());
    assert!(serving.registration.try_lock().is_ok());
    assert!(serving.health_update.try_lock().is_ok());
    assert!(serving.residents.snapshot().unwrap().residents.iter().all(|slot|
        slot.users == 0 && !slot.uncertain && slot.replacement.is_none()));
    drop(state);
    assert_eq!(Arc::strong_count(&ports), 1, "registration left background work");
    Race { ports, first: first.expect("first registration task failed"),
        second: second.expect("second registration task failed"),
        after_first, disjoint_progress }
}

fn assert_success(outcome: &Outcome, kind: Kind, team: &str) {
    assert!(outcome.cleanup_confirmed);
    let body = &outcome.result.as_ref().expect("registration rejected").0;
    assert_eq!(body["status"], if kind == Kind::Login { "signed-in" } else { "imported" });
    assert_eq!(body["team"], team);
    if kind == Kind::Import { assert_eq!(body["serving_allowed"], false); }
}

fn assert_saved_sessions_and_order(ports: &ClaimPorts, kinds: [Kind; 2]) {
    let saved = ports.inner.saved.lock().unwrap();
    assert_eq!(saved.len(), 2);
    for n in 1..=2 {
        assert!(saved.iter().any(|session| session.active_user.0 == format!("user-{n}")
            && session.cookie_header == cookie(n)));
        let calls: Vec<_> = ports.calls.lock().unwrap().iter()
            .filter_map(|&(owner, call)| (owner == n && matches!(call,
                "save" | "grant" | "proxy" | "upsert")).then_some(call)).collect();
        let prefix: &[&str] = if kinds[n as usize - 1] == Kind::Login {
            &["save", "grant", "upsert"]
        } else { &["save", "grant", "proxy", "upsert"] };
        assert!(calls.starts_with(prefix));
    }
}

async fn assert_one_primary(kinds: [Kind; 2]) {
    let race = compete(kinds, Mode::Shared).await;
    assert_success(&race.first, kinds[0], "shared");
    assert!(race.second.cleanup_confirmed);
    assert!(matches!(&race.second.result, Err(ApiError::Internal(error))
        if error.contains("every credited workspace is already in use (shared)")));
    let mut expected = race.ports.initial[1].clone();
    expected.state = AccountState::Exhausted;
    let records = race.ports.inner.records.lock().unwrap();
    assert_eq!(json!(records[1]), json!(expected));
    assert_eq!(json!(records[0]), json!(race.after_first));
    assert_eq!(records.iter().filter(|a|
        a.current_team.as_ref().is_some_and(|team| team.0 == "shared")).count(), 1);
    assert!(race.ports.projects.lock().unwrap().iter().all(|(n, _)| *n == 1));
    assert_eq!(race.ports.calls.lock().unwrap().iter()
        .filter(|&&(n, call)| n == 2 && call == "upsert").count(), 2);
    assert_saved_sessions_and_order(&race.ports, kinds);
}

#[tokio::test]
async fn concurrent_logins_do_not_publish_the_same_primary_team() {
    assert_one_primary([Kind::Login, Kind::Login]).await;
}

#[tokio::test]
async fn concurrent_imports_do_not_publish_the_same_primary_team() {
    assert_one_primary([Kind::Import, Kind::Import]).await;
}

#[tokio::test]
async fn published_login_claim_is_rechecked_by_pending_import() {
    assert_one_primary([Kind::Login, Kind::Import]).await;
}

#[tokio::test]
async fn published_import_claim_is_rechecked_by_pending_login() {
    assert_one_primary([Kind::Import, Kind::Login]).await;
}

#[tokio::test]
async fn losing_primary_candidate_uses_a_lower_ranked_unclaimed_backup() {
    for kinds in [[Kind::Login, Kind::Import], [Kind::Import, Kind::Login]] {
        let race = compete(kinds, Mode::Backup).await;
        assert_success(&race.first, kinds[0], "shared");
        assert_success(&race.second, kinds[1], "metadata-2");
        let records = race.ports.inner.records.lock().unwrap();
        assert_eq!(json!(records[0]), json!(race.after_first));
        assert_eq!(records[1].current_team.as_ref().unwrap().0, "metadata-2");
        assert_eq!(records[1].current_project.as_ref().unwrap().0, "project-metadata-2-2");
        assert_eq!(records[1].credit.unwrap().available, 50.0);
        assert_eq!(records[1].state, AccountState::Ready);
        assert_saved_sessions_and_order(&race.ports, kinds);
        // Later secondary-scope publication is outside this primary-claim slice.
    }
}

#[tokio::test]
async fn disjoint_registration_finishes_while_another_probe_is_blocked() {
    for kinds in [[Kind::Login, Kind::Import], [Kind::Import, Kind::Login]] {
        let race = compete(kinds, Mode::Disjoint).await;
        assert!(race.disjoint_progress, "network probes serialized registration");
        assert_success(&race.first, kinds[0], "credited-1");
        assert_success(&race.second, kinds[1], "credited-2");
        assert_saved_sessions_and_order(&race.ports, kinds);
    }
}

#[tokio::test]
async fn late_claim_with_failed_probe_preserves_the_initial_account() {
    for kinds in [[Kind::Login, Kind::Import], [Kind::Import, Kind::Login]] {
        let race = compete(kinds, Mode::Partial).await;
        assert_success(&race.first, kinds[0], "shared");
        assert!(race.second.cleanup_confirmed);
        assert!(matches!(&race.second.result, Err(ApiError::Internal(error))
            if error == &framer_core::error::CoreError::Framer(PROBE_ERROR.into()).to_string()));
        let records = race.ports.inner.records.lock().unwrap();
        assert_eq!(json!(records[0]), json!(race.after_first));
        assert_eq!(json!(records[1]), json!(race.ports.initial[1]));
        assert!(race.ports.projects.lock().unwrap().iter().all(|(n, _)| *n == 1));
        assert_eq!(race.ports.calls.lock().unwrap().iter()
            .filter(|&&(n, call)| n == 2 && call == "upsert").count(), 1);
        assert_saved_sessions_and_order(&race.ports, kinds);
    }
}
