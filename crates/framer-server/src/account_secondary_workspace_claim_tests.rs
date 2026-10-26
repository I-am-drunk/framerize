//! Inert final-publication races. Every spawned fixture task is joined.
use super::*;
use framer_core::account::{AccountState, WorkspaceScope};
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::{sync::Notify, time::timeout};

const WAIT: Duration = Duration::from_secs(3);
const GET_ERROR: &str = "fixture final subject read failed";
const LIST_ERROR: &str = "fixture final peer read failed";
const WRITE_ERROR: &str = "fixture final scope write failed";
type Outcome = crate::pool_control::CleanupOutcome<Json<Value>>;
type Task = tokio::task::JoinHandle<Outcome>;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind { Login, Import }
#[derive(Clone, Copy, PartialEq)]
enum Mode { Extras, Backup }

fn cookie(n: u128) -> String { format!("session=secondary-fixture-{n}") }
fn credit(available: f64) -> CreditSnapshot {
    CreditSnapshot { available, funds: 1, grants: 1, allocated: available }
}
fn scope(team: &str) -> WorkspaceScope {
    WorkspaceScope { team: TeamId(team.into()), project: ProjectId(format!("project-{team}")) }
}

struct ScopePorts {
    inner: Arc<FixturePorts>,
    mode: Mode,
    create_primary: bool,
    fail_intermediate: bool,
    fault: AtomicUsize,
    store_hold: AtomicUsize,
    store_entered: Notify,
    store_release: Notify,
    probe_once: [AtomicBool; 2],
    probe_entered: [Notify; 2],
    probe_release: [Notify; 2],
    project_entered: [Notify; 2],
    project_release: [Notify; 2],
    writes: Mutex<Vec<ManagedAccount>>,
    calls: Mutex<Vec<(u128, &'static str)>>,
}

fn number(session: &Session) -> u128 {
    let n = session.cookie_header.strip_prefix("session=secondary-fixture-")
        .unwrap().parse().unwrap();
    if !session.active_user.0.is_empty() {
        assert_eq!(session.active_user.0, format!("user-{n}"));
    }
    n
}

impl ScopePorts {
    fn primary(&self, n: u128) -> String { format!("primary-{n}") }
    fn teams(&self, n: u128) -> Vec<(String, f64)> {
        if self.mode == Mode::Backup {
            vec![(self.primary(n), if n == 2 { 50.0 } else { 0.0 }),
                ("shared-primary".into(), 100.0)]
        } else {
            vec![(self.primary(n), 100.0), ("shared-extra".into(), 50.0)]
        }
    }
    fn identity(&self, n: u128) -> Value {
        let spaces: Vec<_> = self.teams(n).iter().map(|(team, _)|
            json!({"scope": "organization", "ownerId": team})).collect();
        json!({"id": format!("user-{n}"), "spaces": spaces})
    }
    fn call(&self, n: u128, name: &'static str) {
        self.calls.lock().unwrap().push((n, name));
    }
    async fn hold_store(&self, operation: usize) {
        if self.store_hold.compare_exchange(operation, 0, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            self.store_entered.notify_one();
            self.store_release.notified().await;
        }
    }
    fn records(&self) -> Vec<ManagedAccount> { self.inner.records.lock().unwrap().clone() }
}

fn scope_fixture(kinds: [Kind; 2], mode: Mode, create: bool, fail: bool)
    -> (AppState, Arc<ScopePorts>) {
    let (_, inner) = fixture(true);
    *inner.records.lock().unwrap() = (1..=2).map(|n| {
        let mut value = account(n);
        value.current_team = Some(TeamId(format!("primary-{n}")));
        value.current_project = None;
        value.credit = Some(credit(7.0));
        value
    }).collect();
    let ports = Arc::new(ScopePorts {
        inner, mode, create_primary: create, fail_intermediate: fail,
        fault: AtomicUsize::new(0), store_hold: AtomicUsize::new(0),
        store_entered: Notify::new(), store_release: Notify::new(),
        probe_once: std::array::from_fn(|_| AtomicBool::new(false)),
        probe_entered: std::array::from_fn(|_| Notify::new()),
        probe_release: std::array::from_fn(|_| Notify::new()),
        project_entered: std::array::from_fn(|_| Notify::new()),
        project_release: std::array::from_fn(|_| Notify::new()),
        writes: Mutex::new(Vec::new()), calls: Mutex::new(Vec::new()),
    });
    let mut allowed: Vec<_> = kinds.iter().enumerate().filter_map(|(i, kind)|
        (*kind == Kind::Login).then_some(i as u128 + 1)).collect();
    if allowed.is_empty() { allowed.push(99); }
    let state = AppState::fixture(ports.clone(), ports.clone(), ports.clone(),
        ports.clone(), policy(&allowed), framer_core::gate::ConcurrencyGate::new());
    (state, ports)
}

#[async_trait::async_trait]
impl StateStore for ScopePorts {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        self.call(id.0.as_u128(), "get");
        self.hold_store(1).await;
        if self.fault.load(Ordering::SeqCst) == 1 { return Err(GET_ERROR.into()); }
        self.inner.get(id).await
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        self.call(0, "list");
        self.hold_store(2).await;
        if self.fault.load(Ordering::SeqCst) == 2 { return Err(LIST_ERROR.into()); }
        self.inner.list().await
    }
    async fn upsert(&self, value: &ManagedAccount) -> Result<(), String> {
        self.call(value.id.0.as_u128(), "upsert");
        let count = {
            let mut writes = self.writes.lock().unwrap();
            writes.push(value.clone());
            writes.iter().filter(|record| record.id == value.id).count()
        };
        self.hold_store(3).await;
        if self.fault.load(Ordering::SeqCst) == 3 { return Err(WRITE_ERROR.into()); }
        if self.fail_intermediate && self.create_primary && count == 3 {
            return Err("fixture intermediate save failed".into());
        }
        self.inner.upsert(value).await
    }
    async fn remove(&self, _: AccountId) -> Result<(), String> { panic!("unexpected remove") }
}

#[async_trait::async_trait]
impl SessionStore for ScopePorts {
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
impl BrowserBridge for ScopePorts {
    async fn auth_complete(&self, id: &AccountId, token: &str) -> Result<(String, Value), String> {
        let n = id.0.as_u128();
        assert_eq!(token, format!("token-{n}"));
        Ok((cookie(n), self.identity(n)))
    }
    async fn register_account_proxy(&self, id: &AccountId, proxy: Option<String>)
        -> Result<(), String> { self.inner.register_account_proxy(id, proxy).await }
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> { panic!("unexpected mint") }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> { panic!("unexpected login") }
}

#[async_trait::async_trait]
impl FramerClient for ScopePorts {
    async fn me(&self, session: &Session) -> Result<Value, String> {
        Ok(self.identity(number(session)))
    }
    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        let n = number(session);
        if self.mode == Mode::Backup && team.0 == "shared-primary"
            && !self.probe_once[n as usize - 1].swap(true, Ordering::SeqCst) {
            self.probe_entered[n as usize - 1].notify_one();
            self.probe_release[n as usize - 1].notified().await;
        }
        let available = self.teams(n).into_iter().find(|(id, _)| *id == team.0).unwrap().1;
        Ok(credit(available))
    }
    async fn claim_workspace_grant(&self, session: &Session) -> Result<bool, String> {
        let n = number(session);
        assert!(self.inner.saved.lock().unwrap().iter().any(|saved|
            saved.active_user == session.active_user && saved.cookie_header == cookie(n)));
        self.call(n, "grant");
        Ok(false)
    }
    async fn team_projects(&self, session: &Session, team: &TeamId) -> Result<Vec<ProjectId>, String> {
        let n = number(session);
        if team.0 == "shared-extra" {
            self.project_entered[n as usize - 1].notify_one();
            self.project_release[n as usize - 1].notified().await;
        }
        if self.create_primary && team.0 == self.primary(n) { return Ok(Vec::new()); }
        Ok(vec![scope(&team.0).project])
    }
    async fn create_project(&self, session: &Session, team: &TeamId) -> Result<ProjectId, String> {
        assert!(self.create_primary);
        assert_eq!(team.0, self.primary(number(session)));
        Ok(ProjectId(format!("created-{}", team.0)))
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
    let body = json!({"account_id": account(n).id, "email_token": format!("token-{n}"),
        "session_cookie": cookie(n), "expected_framer_user_id": format!("user-{n}")});
    match kind {
        Kind::Login => crate::complete_login_observed(State(state),
            Json(serde_json::from_value(body).unwrap())).await,
        Kind::Import => crate::import_account_observed(State(state),
            Json(serde_json::from_value(body).unwrap())).await,
    }
}

async fn join(task: &mut Task) -> Result<Outcome, tokio::task::JoinError> {
    match timeout(WAIT, &mut *task).await {
        Ok(result) => result,
        Err(_) => { task.abort(); task.await }
    }
}

fn assert_settled(state: AppState, ports: &Arc<ScopePorts>) {
    assert!(state.serving_accounts.mutations.lock().unwrap().is_empty());
    assert!(state.serving_accounts.registration.try_lock().is_ok());
    assert!(state.serving_accounts.health_update.try_lock().is_ok());
    assert!(state.serving_accounts.residents.snapshot().unwrap().residents.iter().all(|slot|
        slot.users == 0 && !slot.uncertain && slot.replacement.is_none()));
    drop(state);
    assert_eq!(Arc::strong_count(ports), 1, "unjoined fixture or background task");
}

async fn paused(kind: Kind, create: bool, fail: bool) -> (AppState, Arc<ScopePorts>, Task) {
    let (state, ports) = scope_fixture([kind, Kind::Import], Mode::Extras, create, fail);
    let mut task = tokio::spawn(register(state.clone(), kind, 1));
    if timeout(WAIT, ports.project_entered[0].notified()).await.is_err() {
        task.abort();
        let _ = join(&mut task).await;
        panic!("registration did not reach extra-project preparation");
    }
    (state, ports, task)
}

fn assert_success(outcome: &Outcome, kind: Kind, primary: &str, project: &str) {
    assert!(outcome.cleanup_confirmed);
    let body = &outcome.result.as_ref().expect("registration failed").0;
    assert_eq!(body["status"], if kind == Kind::Login { "signed-in" } else { "imported" });
    assert_eq!(body["team"], primary);
    assert_eq!(body["project"], project);
    if kind == Kind::Import { assert_eq!(body["serving_allowed"], false); }
}

fn assert_error(outcome: &Outcome, expected: &str) {
    assert!(outcome.cleanup_confirmed);
    assert!(matches!(&outcome.result, Err(ApiError::Internal(error)) if error == expected),
        "unexpected result: {:?}", outcome.result);
}

async fn shared_extra_race(kinds: [Kind; 2]) {
    let (state, ports) = scope_fixture(kinds, Mode::Extras, false, false);
    let mut first = tokio::spawn(register(state.clone(), kinds[0], 1));
    let first_ready = timeout(WAIT, ports.project_entered[0].notified()).await.is_ok();
    let mut second = tokio::spawn(register(state.clone(), kinds[1], 2));
    let second_ready = timeout(WAIT, ports.project_entered[1].notified()).await.is_ok();
    let updates_free_during_projects = state.serving_accounts.health_update.try_lock().is_ok();
    ports.project_release[0].notify_one();
    let first = join(&mut first).await;
    let after_first = ports.records()[0].clone();
    ports.project_release[1].notify_one();
    let second = join(&mut second).await;
    assert!(first_ready && second_ready && updates_free_during_projects);
    assert_success(&first.unwrap(), kinds[0], "primary-1", "project-primary-1");
    assert_success(&second.unwrap(), kinds[1], "primary-2", "project-primary-2");
    let records = ports.records();
    assert_eq!(json!(records[0]), json!(after_first));
    assert_eq!(json!(records[0].extra_workspaces), json!([scope("shared-extra")]));
    assert!(records[1].extra_workspaces.is_empty());
    assert_eq!(ports.inner.saved.lock().unwrap().len(), 2);
    assert_settled(state, &ports);
}

#[tokio::test]
async fn concurrent_logins_publish_shared_extra_once() {
    shared_extra_race([Kind::Login, Kind::Login]).await;
}
#[tokio::test]
async fn concurrent_imports_publish_shared_extra_once() {
    shared_extra_race([Kind::Import, Kind::Import]).await;
}
#[tokio::test]
async fn pending_import_rechecks_login_secondary_claim() {
    shared_extra_race([Kind::Login, Kind::Import]).await;
}
#[tokio::test]
async fn pending_login_rechecks_import_secondary_claim() {
    shared_extra_race([Kind::Import, Kind::Login]).await;
}

#[tokio::test]
async fn backup_primary_does_not_republish_winners_team_as_extra() {
    for kinds in [[Kind::Login, Kind::Import], [Kind::Import, Kind::Login]] {
        let (state, ports) = scope_fixture(kinds, Mode::Backup, false, false);
        let mut first = tokio::spawn(register(state.clone(), kinds[0], 1));
        let first_ready = timeout(WAIT, ports.probe_entered[0].notified()).await.is_ok();
        let mut second = tokio::spawn(register(state.clone(), kinds[1], 2));
        let second_ready = timeout(WAIT, ports.probe_entered[1].notified()).await.is_ok();
        ports.probe_release[0].notify_one();
        let first = join(&mut first).await;
        ports.probe_release[1].notify_one();
        let second = join(&mut second).await;
        assert!(first_ready && second_ready);
        assert_success(&first.unwrap(), kinds[0], "shared-primary", "project-shared-primary");
        assert_success(&second.unwrap(), kinds[1], "primary-2", "project-primary-2");
        let records = ports.records();
        assert!(records.iter().all(|record| record.extra_workspaces.is_empty()));
        assert_ne!(records[0].current_team, records[1].current_team);
        assert_settled(state, &ports);
    }
}

async fn preserves_fresh_state(kind: Kind) {
    let (state, ports, mut task) = paused(kind, false, false).await;
    let mut expected = {
        let _update = state.serving_accounts.health_update.lock().await;
        let mut records = ports.inner.records.lock().unwrap();
        let current = &mut records[0];
        current.alias = "fresh alias".into();
        current.email = Some("fresh@example.invalid".into());
        current.state = AccountState::CoolingDown;
        current.credit = Some(credit(13.0));
        current.consecutive_failures = 4;
        current.cooldown_step = 3;
        current.cooling_until_ms = Some(98_765);
        current.clone()
    };
    ports.project_release[0].notify_one();
    let outcome = join(&mut task).await.unwrap();
    assert_success(&outcome, kind, "primary-1", "project-primary-1");
    expected.current_project = Some(ProjectId("project-primary-1".into()));
    expected.extra_workspaces = vec![scope("shared-extra")];
    assert_eq!(json!(ports.records()[0]), json!(expected));
    if kind == Kind::Login { assert_eq!(outcome.result.unwrap().0["credits"], 13.0); }
    assert_settled(state, &ports);
}

#[tokio::test]
async fn login_adoption_preserves_fresh_health_and_unrelated_fields() {
    preserves_fresh_state(Kind::Login).await;
}
#[tokio::test]
async fn import_adoption_preserves_fresh_health_and_unrelated_fields() {
    preserves_fresh_state(Kind::Import).await;
}

async fn import_creation(fail_intermediate: bool) {
    let (state, ports, mut task) = paused(Kind::Import, true, fail_intermediate).await;
    let before = ports.records();
    assert_eq!(ports.writes.lock().unwrap().len(), 3);
    let expected_saved = (!fail_intermediate).then(|| ProjectId("created-primary-1".into()));
    assert_eq!(before[0].current_project, expected_saved);
    let attempts = ports.writes.lock().unwrap().clone();
    assert_eq!(attempts[2].current_project, Some(ProjectId("created-primary-1".into())));
    ports.project_release[0].notify_one();
    let outcome = join(&mut task).await.unwrap();
    assert_success(&outcome, Kind::Import, "primary-1", "created-primary-1");
    let mut expected = before[0].clone();
    expected.current_project = Some(ProjectId("created-primary-1".into()));
    expected.extra_workspaces = vec![scope("shared-extra")];
    assert_eq!(json!(ports.records()), json!([expected, before[1]]));
    assert_eq!(ports.writes.lock().unwrap().len(), 4);
    assert_settled(state, &ports);
}

#[tokio::test]
async fn import_creation_advances_expected_scope_after_successful_save() {
    import_creation(false).await;
}
#[tokio::test]
async fn import_creation_keeps_expected_scope_after_failed_save() {
    import_creation(true).await;
}

async fn final_error(kind: Kind, fault: usize) {
    let (state, ports, mut task) = paused(kind, false, false).await;
    let before = ports.records();
    ports.fault.store(fault, Ordering::SeqCst);
    ports.project_release[0].notify_one();
    let outcome = join(&mut task).await.unwrap();
    assert_error(&outcome, [GET_ERROR, LIST_ERROR, WRITE_ERROR][fault - 1]);
    assert_eq!(json!(ports.records()), json!(before));
    let writes = ports.writes.lock().unwrap();
    assert_eq!(writes.len(), if fault == 3 { 3 } else { 2 });
    assert!(writes.iter().all(|record| record.state != AccountState::Exhausted));
    drop(writes);
    assert_eq!(ports.inner.saved.lock().unwrap().len(), 1);
    assert_settled(state, &ports);
}

#[tokio::test]
async fn login_final_store_errors_preserve_persisted_record() {
    for fault in 1..=3 { final_error(Kind::Login, fault).await; }
}
#[tokio::test]
async fn import_final_store_errors_preserve_persisted_record() {
    for fault in 1..=3 { final_error(Kind::Import, fault).await; }
}

#[tokio::test]
async fn failed_publication_does_not_replace_the_prepared_caller() {
    for fault in 1..=3 {
        let (state, ports) = scope_fixture([Kind::Import; 2], Mode::Extras, false, false);
        let saved = ports.records();
        let expected = saved[0].clone();
        let mut prepared = expected.clone();
        prepared.current_project = Some(ProjectId("prepared-project".into()));
        prepared.extra_workspaces = vec![scope("prepared-extra")];
        let before = json!(prepared);
        ports.fault.store(fault, Ordering::SeqCst);
        let result = crate::account_scope_publication::publish(&state, &expected, &mut prepared).await;
        assert!(matches!(result, Err(ApiError::Internal(error))
            if error == [GET_ERROR, LIST_ERROR, WRITE_ERROR][fault - 1]));
        assert_eq!(json!(prepared), before);
        assert_eq!(json!(ports.records()), json!(saved));
        assert_settled(state, &ports);
    }
}

fn change_scope(current: &mut ManagedAccount, field: usize) {
    match field {
        0 => current.framer_user_id = FramerUserId("different-user".into()),
        1 => current.castle_device_id = "different-device".into(),
        2 => current.current_team = Some(TeamId("different-team".into())),
        3 => current.current_project = Some(ProjectId("different-project".into())),
        4 => current.proxy = Some("http://different-proxy.invalid:8080".into()),
        _ => unreachable!(),
    }
}

async fn changed_scope(kind: Kind, field: usize) {
    let (state, ports, mut task) = paused(kind, false, false).await;
    {
        let _update = state.serving_accounts.health_update.lock().await;
        let mut records = ports.inner.records.lock().unwrap();
        if field == 5 { records.remove(0); }
        else { change_scope(&mut records[0], field); }
    }
    let before = ports.records();
    // Validation must precede even a failing peer read.
    ports.fault.store(2, Ordering::SeqCst);
    ports.project_release[0].notify_one();
    let outcome = join(&mut task).await.unwrap();
    let message = if field == 5 { "account removed during update" }
        else { "account scope changed during update" };
    assert_error(&outcome, &format!("account not eligible: {message}"));
    assert_eq!(json!(ports.records()), json!(before));
    assert_eq!(ports.writes.lock().unwrap().len(), 2);
    assert_settled(state, &ports);
}

#[tokio::test]
async fn login_final_publication_checks_all_five_persisted_scope_fields() {
    for field in 0..5 { changed_scope(Kind::Login, field).await; }
}
#[tokio::test]
async fn import_final_publication_checks_all_five_persisted_scope_fields() {
    for field in 0..5 { changed_scope(Kind::Import, field).await; }
}
#[tokio::test]
async fn removed_subject_refuses_both_callers_before_peer_read() {
    for kind in [Kind::Login, Kind::Import] { changed_scope(kind, 5).await; }
}

async fn peer_filter(kind: Kind, variant: usize) {
    let (state, ports, mut task) = paused(kind, false, false).await;
    {
        let _update = state.serving_accounts.health_update.lock().await;
        let mut records = ports.inner.records.lock().unwrap();
        records[0].extra_workspaces = vec![scope("shared-extra")];
        let peer = &mut records[1];
        peer.current_project = None;
        match variant {
            0 => peer.current_team = Some(TeamId("shared-extra".into())),
            1 => peer.extra_workspaces = vec![scope("shared-extra")],
            2 => {
                peer.current_team = Some(TeamId("shared-extra-neighbor".into()));
                peer.extra_workspaces = vec![scope("shared-extra-other")];
            }
            _ => unreachable!(),
        }
    }
    let mut expected = ports.records();
    expected[0].current_project = Some(ProjectId("project-primary-1".into()));
    if variant != 2 { expected[0].extra_workspaces.clear(); }
    ports.project_release[0].notify_one();
    let outcome = join(&mut task).await.unwrap();
    assert_success(&outcome, kind, "primary-1", "project-primary-1");
    assert_eq!(json!(ports.records()), json!(expected));
    assert_settled(state, &ports);
}

#[tokio::test]
async fn final_publication_filters_peer_primary_without_project() {
    for kind in [Kind::Login, Kind::Import] { peer_filter(kind, 0).await; }
}
#[tokio::test]
async fn final_publication_filters_peer_extra_team() {
    for kind in [Kind::Login, Kind::Import] { peer_filter(kind, 1).await; }
}
#[tokio::test]
async fn current_account_and_full_identity_controls_keep_unclaimed_scope() {
    for kind in [Kind::Login, Kind::Import] { peer_filter(kind, 2).await; }
}

#[tokio::test]
async fn cancellation_during_project_preparation_joins_without_final_publication() {
    for kind in [Kind::Login, Kind::Import] {
        let (state, ports, mut task) = paused(kind, false, false).await;
        let before = ports.records();
        task.abort();
        let cancelled = join(&mut task).await.err().is_some_and(|error| error.is_cancelled());
        assert!(cancelled);
        assert_eq!(json!(ports.records()), json!(before));
        assert_eq!(ports.writes.lock().unwrap().len(), 2);
        assert_settled(state, &ports);
    }
}

#[tokio::test]
async fn cancelled_store_wait_preserves_caller_and_releases_publication_lock() {
    for operation in 1..=3 {
        let (state, ports) = scope_fixture([Kind::Import; 2], Mode::Extras, false, false);
        let saved = ports.records();
        let expected = saved[0].clone();
        let mut prepared = expected.clone();
        prepared.current_project = Some(ProjectId("prepared-project".into()));
        prepared.extra_workspaces = vec![scope("shared-extra")];
        let before = json!(prepared);
        ports.store_hold.store(operation, Ordering::SeqCst);
        let mut publication = Box::pin(crate::account_scope_publication::publish(
            &state, &expected, &mut prepared));
        assert!(matches!(futures::poll!(publication.as_mut()), std::task::Poll::Pending));
        assert_eq!(ports.store_hold.load(Ordering::SeqCst), 0);
        assert!(state.serving_accounts.health_update.try_lock().is_err());
        drop(publication);
        assert_eq!(json!(prepared), before);
        assert_eq!(json!(ports.records()), json!(saved));
        assert_settled(state, &ports);
    }
}

#[tokio::test]
async fn peer_recheck_and_save_share_the_primary_admission_mutex() {
    let (state, ports) = scope_fixture([Kind::Import; 2], Mode::Extras, false, false);
    let saved = ports.records();
    let mut first_prepared = saved[0].clone();
    let mut second_prepared = saved[1].clone();
    first_prepared.extra_workspaces = vec![scope("shared-extra")];
    second_prepared.extra_workspaces = vec![scope("shared-extra")];
    ports.store_hold.store(3, Ordering::SeqCst);
    let mut first = Box::pin(crate::account_scope_publication::publish(
        &state, &saved[0], &mut first_prepared));
    assert!(matches!(futures::poll!(first.as_mut()), std::task::Poll::Pending));
    assert_eq!(ports.store_hold.load(Ordering::SeqCst), 0);
    let mut second = Box::pin(crate::account_scope_publication::publish(
        &state, &saved[1], &mut second_prepared));
    let second_poll = futures::poll!(second.as_mut());
    let was_waiting = second_poll.is_pending();
    let subject_read_early = ports.calls.lock().unwrap().iter()
        .any(|&(id, call)| id == 2 && call == "get");
    ports.store_release.notify_one();
    timeout(WAIT, first).await.unwrap().unwrap();
    match second_poll {
        std::task::Poll::Ready(result) => result.unwrap(),
        std::task::Poll::Pending => timeout(WAIT, second.as_mut()).await.unwrap().unwrap(),
    }
    drop(second);
    assert!(was_waiting && !subject_read_early);
    assert_eq!(json!(first_prepared.extra_workspaces), json!([scope("shared-extra")]));
    assert!(second_prepared.extra_workspaces.is_empty());
    assert_eq!(json!(ports.records()), json!([first_prepared, second_prepared]));
    assert_settled(state, &ports);
}
