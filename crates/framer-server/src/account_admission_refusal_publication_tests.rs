//! Inert callers retain a newer subject when admission cannot publish.
use super::*;
use framer_core::account::{AccountState, WorkspaceScope};
use tokio::{sync::Notify, task::JoinHandle, time::{timeout, Duration}};

const COOKIE: &str = "session=fixture-only";
const READ_ERROR: &str = "fixture refusal read failed";
const WRITE_ERROR: &str = "fixture refusal write failed";
const WAIT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case { Changed(usize), Removed, Fresh, ReadError, WriteError }

struct RefusalPorts {
    inner: Arc<FixturePorts>,
    case: Case,
    calls: Mutex<Vec<&'static str>>,
    writes: Mutex<Vec<ManagedAccount>>,
    entered: Notify,
    release: Notify,
}

impl RefusalPorts {
    fn record(&self, call: &'static str) { self.calls.lock().unwrap().push(call); }
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

#[async_trait::async_trait]
impl BrowserBridge for RefusalPorts {
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
impl SessionStore for RefusalPorts {
    async fn save(&self, session: &Session) -> Result<(), String> {
        self.assert_session(session);
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
impl StateStore for RefusalPorts {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        if self.case == Case::ReadError && self.inner.state_writes.load(Ordering::SeqCst) == 1 {
            self.record("mark_read_error");
            return Err(READ_ERROR.into());
        }
        self.record("get");
        self.inner.get(id).await
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        self.record("list");
        self.inner.list().await
    }
    async fn remove(&self, _: AccountId) -> Result<(), String> {
        self.inner.unexpected("state");
    }
    async fn upsert(&self, current: &ManagedAccount) -> Result<(), String> {
        let attempt = {
            let mut writes = self.writes.lock().unwrap();
            writes.push(current.clone());
            writes.len()
        };
        if self.case == Case::WriteError && attempt == 2 {
            self.record("mark_write_error");
            return Err(WRITE_ERROR.into());
        }
        self.record("upsert");
        self.inner.upsert(current).await
    }
}

#[async_trait::async_trait]
impl FramerClient for RefusalPorts {
    async fn me(&self, session: &Session) -> Result<Value, String> {
        assert_eq!(session.cookie_header, COOKIE);
        if session.active_user.0.is_empty() { self.record("probe"); }
        else { self.assert_session(session); self.record("me"); }
        Ok(self.identity())
    }
    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        self.assert_session(session);
        assert_eq!(team.0, "team-2");
        self.record("usage");
        self.entered.notify_one();
        self.release.notified().await;
        self.inner.usage(session, team).await
    }
    async fn claim_workspace_grant(&self, session: &Session) -> Result<bool, String> {
        self.assert_session(session);
        assert_eq!(self.inner.saved.lock().unwrap().len(), 1);
        self.record("grant");
        Ok(false)
    }
    async fn team_projects(&self, _: &Session, _: &TeamId) -> Result<Vec<ProjectId>, String> {
        self.inner.unexpected("client");
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

fn refusal_fixture(case: Case) -> (AppState, Arc<RefusalPorts>) {
    let (_, inner) = fixture(true);
    let available = if matches!(case, Case::Changed(_) | Case::Removed) { 100.0 } else { 0.0 };
    *inner.workspaces.lock().unwrap() = vec![("team-2", available)];
    let mut current = account(2);
    current.alias = "refusal fixture".into();
    current.email = None;
    current.current_project = Some(ProjectId("saved-project".into()));
    current.credit = Some(credit(7.0));
    current.extra_workspaces = vec![WorkspaceScope {
        team: TeamId("saved-extra".into()), project: ProjectId("saved-extra-project".into()),
    }];
    *inner.records.lock().unwrap() = vec![account(1), current];
    let ports = Arc::new(RefusalPorts {
        inner, case, calls: Mutex::new(Vec::new()), writes: Mutex::new(Vec::new()),
        entered: Notify::new(), release: Notify::new(),
    });
    let state = AppState::fixture(
        ports.clone(), ports.clone(), ports.clone(), ports.clone(),
        policy(&[2]), framer_core::gate::ConcurrencyGate::new(),
    );
    (state, ports)
}

type Outcome = crate::pool_control::CleanupOutcome<Json<Value>>;
type Task = JoinHandle<Outcome>;

async fn register(state: AppState, login: bool) -> Outcome {
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

async fn join(task: &mut Task) -> Result<Outcome, tokio::task::JoinError> {
    match timeout(WAIT, &mut *task).await {
        Ok(result) => result,
        Err(_) => {
            task.abort();
            let _ = task.await;
            panic!("refusal fixture task did not settle");
        }
    }
}

async fn abort_fixture(task: &mut Task, reason: &str) -> ! {
    task.abort();
    let _ = task.await;
    panic!("{reason}");
}

fn change_saved(records: &mut Vec<ManagedAccount>, case: Case) -> bool {
    if case == Case::Removed {
        let before = records.len();
        records.retain(|current| current.id != account(2).id);
        return records.len() + 1 == before;
    }
    let Some(current) = records.iter_mut().find(|current| current.id == account(2).id)
        else { return false; };
    current.alias = "fresh alias".into();
    current.email = Some("fresh@example.invalid".into());
    current.state = AccountState::CoolingDown;
    current.credit = Some(credit(19.0));
    current.consecutive_failures = 6;
    current.cooldown_step = 4;
    current.cooling_until_ms = Some(98_765);
    current.extra_workspaces.push(WorkspaceScope {
        team: TeamId("fresh-extra".into()), project: ProjectId("fresh-extra-project".into()),
    });
    if let Case::Changed(field) = case {
        match field {
            0 => current.framer_user_id = FramerUserId("replacement-user".into()),
            1 => current.castle_device_id = "replacement-device".into(),
            2 => current.current_team = Some(TeamId("replacement-team".into())),
            3 => current.current_project = Some(ProjectId("replacement-project".into())),
            4 => current.proxy = Some("http://replacement.invalid:8080".into()),
            _ => unreachable!(),
        }
    }
    true
}

async fn exercise(login: bool, case: Case) {
    let (state, ports) = refusal_fixture(case);
    let serving = state.serving_accounts.clone();
    let config = serving.config();
    let initial = ports.inner.records.lock().unwrap().clone();
    let mut task = tokio::spawn(register(state.clone(), login));
    if timeout(WAIT, ports.entered.notified()).await.is_err() {
        abort_fixture(&mut task, "admission probe was not reached").await;
    }
    let update = match timeout(WAIT, serving.health_update.lock()).await {
        Ok(update) => update,
        Err(_) => abort_fixture(&mut task, "probe held the publication lock").await,
    };
    let (before_mark, changed) = {
        let mut records = ports.inner.records.lock().unwrap();
        let changed = change_saved(&mut records, case);
        (records.clone(), changed)
    };
    drop(update);
    ports.release.notify_one();
    let outcome = join(&mut task).await.unwrap();
    drop(task);
    assert!(changed, "initial subject was not present");
    assert!(outcome.cleanup_confirmed);
    assert_error(outcome.result, case);
    assert_records(&ports, &initial, &before_mark, case);
    assert_calls(&ports, login, case);
    assert_eq!(serving.config(), config);
    assert_settled(state, &ports, login);
}

fn assert_error(result: Result<Json<Value>, ApiError>, case: Case) {
    let expected = match case {
        Case::Changed(_) => "account not eligible: account scope changed during update",
        Case::Removed => "account not eligible: account removed during update",
        Case::Fresh => "account not eligible: refusal fixture: no workspace has credits \u{2014} account cannot serve requests",
        Case::ReadError => READ_ERROR,
        Case::WriteError => WRITE_ERROR,
    };
    match result {
        Err(ApiError::Internal(error)) => assert_eq!(error, expected),
        _ => panic!("expected the exact admission or marking-store error"),
    }
}

fn assert_records(
    ports: &RefusalPorts, initial: &[ManagedAccount], before: &[ManagedAccount], case: Case,
) {
    let mut expected = before.to_vec();
    if case == Case::Fresh {
        expected.iter_mut().find(|record| record.id == account(2).id).unwrap().state
            = AccountState::Exhausted;
    }
    assert_eq!(json!(*ports.inner.records.lock().unwrap()), json!(expected));
    let writes = ports.writes.lock().unwrap();
    let attempted_mark = matches!(case, Case::Fresh | Case::WriteError);
    assert_eq!(writes.len(), if attempted_mark { 2 } else { 1 });
    let original = initial.iter().find(|record| record.id == account(2).id).unwrap();
    assert_eq!(json!(writes[0]), json!(original));
    if attempted_mark {
        let mut mark = before.iter().find(|record| record.id == account(2).id).unwrap().clone();
        mark.state = AccountState::Exhausted;
        assert_eq!(json!(writes[1]), json!(mark));
    }
    assert_eq!(ports.inner.state_writes.load(Ordering::SeqCst),
        if case == Case::Fresh { 2 } else { 1 });
}

fn assert_calls(ports: &RefusalPorts, login: bool, case: Case) {
    let mut expected = vec![if login { "auth_complete" } else { "probe" },
        "get", "save", "grant"];
    if !login { expected.push("proxy"); }
    expected.extend(["upsert", "list", "me", "usage"]);
    match case {
        Case::Changed(_) | Case::Removed => expected.extend(["get", "get"]),
        Case::Fresh => expected.extend(["get", "upsert"]),
        Case::ReadError => expected.push("mark_read_error"),
        Case::WriteError => expected.extend(["get", "mark_write_error"]),
    }
    assert_eq!(*ports.calls.lock().unwrap(), expected);
}

fn assert_settled(state: AppState, ports: &Arc<RefusalPorts>, login: bool) {
    let saved = ports.inner.saved.lock().unwrap();
    assert_eq!(saved.len(), 1);
    ports.assert_session(&saved[0]);
    drop(saved);
    let proxies = if login { Vec::new() } else { vec![(account(2).id, account(2).proxy)] };
    assert_eq!(*ports.inner.registered_proxies.lock().unwrap(), proxies);
    assert_eq!(ports.inner.browser_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.inner.client_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.inner.session_calls.load(Ordering::SeqCst), 1);
    assert!(state.serving_accounts.mutations.lock().unwrap().is_empty());
    assert!(state.serving_accounts.registration.try_lock().is_ok());
    assert!(state.serving_accounts.health_update.try_lock().is_ok());
    assert!(state.serving_accounts.residents.snapshot().unwrap().residents.iter()
        .all(|slot| slot.users == 0 && !slot.uncertain && slot.replacement.is_none()));
    drop(state);
    assert_eq!(Arc::strong_count(ports), 1, "background work retained fixture ports");
}

#[tokio::test]
async fn login_scope_refusal_preserves_all_five_changed_fields() {
    for field in 0..5 { exercise(true, Case::Changed(field)).await; }
}

#[tokio::test]
async fn import_scope_refusal_preserves_all_five_changed_fields() {
    for field in 0..5 { exercise(false, Case::Changed(field)).await; }
}

#[tokio::test]
async fn login_removed_subject_is_not_recreated_after_refusal() {
    exercise(true, Case::Removed).await;
}

#[tokio::test]
async fn import_removed_subject_is_not_recreated_after_refusal() {
    exercise(false, Case::Removed).await;
}

#[tokio::test]
async fn login_credit_refusal_preserves_fresh_unrelated_fields() {
    exercise(true, Case::Fresh).await;
}

#[tokio::test]
async fn import_credit_refusal_preserves_fresh_unrelated_fields() {
    exercise(false, Case::Fresh).await;
}

#[tokio::test]
async fn login_refusal_mark_read_error_preserves_saved_record() {
    exercise(true, Case::ReadError).await;
}

#[tokio::test]
async fn import_refusal_mark_read_error_preserves_saved_record() {
    exercise(false, Case::ReadError).await;
}

#[tokio::test]
async fn login_refusal_mark_write_error_preserves_saved_record() {
    exercise(true, Case::WriteError).await;
}

#[tokio::test]
async fn import_refusal_mark_write_error_preserves_saved_record() {
    exercise(false, Case::WriteError).await;
}
