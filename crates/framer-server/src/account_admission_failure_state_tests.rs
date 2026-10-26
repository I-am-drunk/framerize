//! Inert caller coverage: failed admission is not evidence of exhausted credits.
use super::*;
use framer_core::account::{AccountState, WorkspaceScope};

const COOKIE: &str = "session=fixture-only";
const PROFILE_ERROR: &str = "fixture profile read failed";
const READ_ERROR: &str = "fixture admission state read failed";
const WRITE_ERROR: &str = "fixture admission state write failed";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    ProfileError,
    UpdateReadError,
    UpdateWriteError,
    Refused,
    Success,
}

struct AdmissionPorts {
    inner: Arc<FixturePorts>,
    case: Case,
    calls: Mutex<Vec<&'static str>>,
    write_attempts: Mutex<Vec<ManagedAccount>>,
}

impl AdmissionPorts {
    fn record(&self, call: &'static str) {
        self.calls.lock().unwrap().push(call);
    }

    fn assert_session(&self, session: &Session) {
        assert_eq!(session.active_user.0, "user-2");
        assert_eq!(session.cookie_header, COOKIE);
    }

    fn identity(&self) -> Value {
        json!({"id": "user-2", "spaces": [
            {"scope": "organization", "ownerId": "team-2"}
        ]})
    }
}

#[async_trait::async_trait]
impl BrowserBridge for AdmissionPorts {
    async fn auth_complete(&self, id: &AccountId, token: &str) -> Result<(String, Value), String> {
        assert_eq!(*id, account(2).id);
        assert_eq!(token, "fixture-token");
        self.record("auth_complete");
        Ok((COOKIE.into(), self.identity()))
    }

    async fn register_account_proxy(
        &self,
        id: &AccountId,
        proxy: Option<String>,
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
impl SessionStore for AdmissionPorts {
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
impl StateStore for AdmissionPorts {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        if self.case == Case::UpdateReadError && self.inner.state_writes.load(Ordering::SeqCst) == 1
        {
            self.record("update_read_error");
            return Err(READ_ERROR.into());
        }
        self.record("get");
        self.inner.get(id).await
    }

    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        self.record("list");
        self.inner.list().await
    }

    async fn upsert(&self, current: &ManagedAccount) -> Result<(), String> {
        let attempt = {
            let mut writes = self.write_attempts.lock().unwrap();
            writes.push(current.clone());
            writes.len()
        };
        if self.case == Case::UpdateWriteError && attempt == 2 {
            self.record("update_write_error");
            return Err(WRITE_ERROR.into());
        }
        self.record("upsert");
        self.inner.upsert(current).await
    }

    async fn remove(&self, _: AccountId) -> Result<(), String> {
        self.inner.unexpected("state");
    }
}

#[async_trait::async_trait]
impl FramerClient for AdmissionPorts {
    async fn me(&self, session: &Session) -> Result<Value, String> {
        assert_eq!(session.cookie_header, COOKIE);
        if session.active_user.0.is_empty() {
            self.record("probe");
        } else {
            self.assert_session(session);
            self.record("me");
            if self.case == Case::ProfileError {
                return Err(PROFILE_ERROR.into());
            }
        }
        Ok(self.identity())
    }

    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        self.assert_session(session);
        assert_eq!(team.0, "team-2");
        self.record("usage");
        self.inner.usage(session, team).await
    }

    async fn claim_workspace_grant(&self, session: &Session) -> Result<bool, String> {
        self.assert_session(session);
        assert_eq!(self.inner.saved.lock().unwrap().len(), 1);
        self.record("grant");
        Ok(false)
    }

    async fn team_projects(
        &self,
        session: &Session,
        team: &TeamId,
    ) -> Result<Vec<ProjectId>, String> {
        self.assert_session(session);
        assert_eq!(self.case, Case::Success);
        assert_eq!(team.0, "team-2");
        self.record("projects");
        Ok(vec![ProjectId("adopted-project".into())])
    }

    async fn create_project(&self, _: &Session, _: &TeamId) -> Result<ProjectId, String> {
        self.inner.unexpected("client");
    }

    async fn request_magic_link(&self, _: &str, _: &CastleToken) -> Result<(), String> {
        self.inner.unexpected("client");
    }

    async fn complete_magic_link(&self, _: &str, _: &CastleToken) -> Result<Session, String> {
        self.inner.unexpected("client");
    }

    async fn create_workspace(&self, _: &Session, _: &WorkspaceSpec) -> Result<TeamId, String> {
        self.inner.unexpected("client");
    }

    async fn run_inference(
        &self,
        _: &Session,
        _: &dyn BrowserBridge,
        _: &AccountId,
        _: &ProjectId,
        _: &InferenceRequest,
    ) -> Result<InferenceResult, String> {
        self.inner.unexpected("client");
    }

    async fn proxy(
        &self,
        _: &Session,
        _: &dyn BrowserBridge,
        _: &AccountId,
        _: &ProjectId,
        _: &str,
        _: Value,
        _: &str,
    ) -> Result<ProxyResponse, String> {
        self.inner.unexpected("client");
    }

    async fn freechat_long(
        &self,
        _: &Session,
        _: &dyn BrowserBridge,
        _: &AccountId,
        _: &ProjectId,
        _: Value,
        _: &ChainConfig,
        _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
    ) -> Result<AssembledResponse, String> {
        self.inner.unexpected("client");
    }

    async fn native_long(
        &self,
        _: &Session,
        _: &dyn BrowserBridge,
        _: &AccountId,
        _: &ProjectId,
        _: Value,
        _: &ChainConfig,
        _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
    ) -> Result<AssembledResponses, String> {
        self.inner.unexpected("client");
    }
}

fn credit(available: f64) -> CreditSnapshot {
    CreditSnapshot {
        available,
        funds: 1,
        grants: 1,
        allocated: available,
    }
}

fn admission_fixture(login: bool, case: Case) -> (AppState, Arc<AdmissionPorts>, ManagedAccount) {
    let (_, inner) = fixture(true);
    let available = if case == Case::Refused { 0.0 } else { 100.0 };
    *inner.workspaces.lock().unwrap() = vec![("team-2", available)];
    let mut initial = account(2);
    initial.current_project = None;
    initial.credit = Some(credit(7.0));
    initial.consecutive_failures = 3;
    initial.cooldown_step = 2;
    initial.cooling_until_ms = Some(12_345);
    initial.extra_workspaces.push(WorkspaceScope {
        team: TeamId("saved-extra".into()),
        project: ProjectId("saved-extra-project".into()),
    });
    *inner.records.lock().unwrap() = vec![account(1), initial.clone()];
    let ports = Arc::new(AdmissionPorts {
        inner,
        case,
        calls: Mutex::new(Vec::new()),
        write_attempts: Mutex::new(Vec::new()),
    });
    // Failure paths allow serving, so early return must prevent any prewarm.
    // The successful import control excludes serving without blocking registration.
    let allowed: &[u128] = if !login && case == Case::Success {
        &[1]
    } else {
        &[2]
    };
    let state = AppState::fixture(
        ports.clone(),
        ports.clone(),
        ports.clone(),
        ports.clone(),
        policy(allowed),
        framer_core::gate::ConcurrencyGate::new(),
    );
    (state, ports, initial)
}

async fn exercise(login: bool, case: Case) {
    let (state, ports, initial) = admission_fixture(login, case);
    let serving = state.serving_accounts.clone();
    let config = serving.config();
    let outcome = if login {
        let body = serde_json::from_value(json!({
            "account_id": initial.id, "email_token": "fixture-token"
        }))
        .unwrap();
        crate::complete_login_observed(State(state), Json(body)).await
    } else {
        let body = serde_json::from_value(json!({
            "account_id": initial.id, "session_cookie": COOKIE,
            "expected_framer_user_id": "user-2"
        }))
        .unwrap();
        crate::import_account_observed(State(state), Json(body)).await
    };
    assert!(outcome.cleanup_confirmed);
    assert_result(outcome.result, &initial, login, case);
    assert_saved(&ports, &initial, login);
    assert_calls(&ports, login);
    assert_eq!(serving.config(), config);
    assert!(serving.mutations.lock().unwrap().is_empty());
    assert!(serving.registration.try_lock().is_ok());
    assert!(serving.health_update.try_lock().is_ok());
    assert!(serving
        .residents
        .snapshot()
        .unwrap()
        .residents
        .iter()
        .all(|slot| { slot.users == 0 && !slot.uncertain && slot.replacement.is_none() }));
    assert_eq!(
        Arc::strong_count(&ports),
        1,
        "background work retained ports"
    );
}

fn assert_result(
    result: Result<Json<Value>, ApiError>,
    initial: &ManagedAccount,
    login: bool,
    case: Case,
) {
    if case == Case::Success {
        let response = result.unwrap().0;
        assert_eq!(
            response["status"],
            if login { "signed-in" } else { "imported" }
        );
        assert_eq!(response["account_id"], json!(initial.id));
        assert_eq!(response["framer_user_id"], json!(initial.framer_user_id));
        assert_eq!(response["team"], "team-2");
        assert_eq!(response["project"], "adopted-project");
        if !login {
            assert_eq!(response["serving_allowed"], false);
        }
        return;
    }
    let expected = match case {
        Case::ProfileError => format!("framer: {PROFILE_ERROR}"),
        Case::UpdateReadError => format!("store: {READ_ERROR}"),
        Case::UpdateWriteError => format!("store: {WRITE_ERROR}"),
        Case::Refused => format!(
            "account not eligible: {}: no workspace has credits \u{2014} account cannot serve requests",
            initial.email.as_ref().unwrap()
        ),
        Case::Success => unreachable!(),
    };
    match result {
        Err(ApiError::Internal(error)) => assert_eq!(error, expected),
        other => panic!("unexpected admission result: {other:?}"),
    }
}

fn assert_saved(ports: &AdmissionPorts, initial: &ManagedAccount, login: bool) {
    let mut expected = initial.clone();
    let mut attempts = vec![initial.clone()];
    let persisted = match ports.case {
        Case::UpdateWriteError => {
            let mut attempted = initial.clone();
            attempted.credit = Some(credit(100.0));
            attempts.push(attempted);
            1
        }
        Case::Refused => {
            expected.state = AccountState::Exhausted;
            attempts.push(expected.clone());
            2
        }
        Case::Success => {
            expected.credit = Some(credit(100.0));
            attempts.push(expected.clone());
            expected.current_project = Some(ProjectId("adopted-project".into()));
            expected.extra_workspaces.clear();
            attempts.push(expected.clone());
            3
        }
        _ => 1,
    };
    assert_eq!(
        serde_json::to_value(&*ports.inner.records.lock().unwrap()).unwrap(),
        json!([account(1), expected])
    );
    assert_eq!(
        serde_json::to_value(&*ports.write_attempts.lock().unwrap()).unwrap(),
        json!(attempts)
    );
    assert_eq!(ports.inner.state_writes.load(Ordering::SeqCst), persisted);
    assert_eq!(ports.inner.session_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.inner.browser_calls.load(Ordering::SeqCst), 0);
    let saved = ports.inner.saved.lock().unwrap();
    assert_eq!(saved.len(), 1);
    ports.assert_session(&saved[0]);
    let proxies = if login {
        Vec::new()
    } else {
        vec![(initial.id, initial.proxy.clone())]
    };
    assert_eq!(*ports.inner.registered_proxies.lock().unwrap(), proxies);
}

fn assert_calls(ports: &AdmissionPorts, login: bool) {
    let mut expected = vec![
        if login { "auth_complete" } else { "probe" },
        "get",
        "save",
        "grant",
    ];
    if !login {
        expected.push("proxy");
    }
    expected.extend(["upsert", "list", "me"]);
    match ports.case {
        Case::ProfileError => {}
        Case::UpdateReadError => expected.extend(["usage", "update_read_error"]),
        Case::UpdateWriteError => expected.extend(["usage", "get", "update_write_error"]),
        Case::Refused => expected.extend(["usage", "upsert"]),
        Case::Success => expected.extend([
            "usage", "get", "upsert", "projects", "me", "usage", "upsert",
        ]),
    }
    assert_eq!(*ports.calls.lock().unwrap(), expected);
}

#[tokio::test(flavor = "current_thread")]
async fn login_profile_error_preserves_initial_record() {
    exercise(true, Case::ProfileError).await;
}

#[tokio::test(flavor = "current_thread")]
async fn import_profile_error_preserves_initial_record() {
    exercise(false, Case::ProfileError).await;
}

#[tokio::test(flavor = "current_thread")]
async fn login_admission_update_error_preserves_initial_record() {
    for case in [Case::UpdateReadError, Case::UpdateWriteError] {
        exercise(true, case).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn import_admission_update_error_preserves_initial_record() {
    for case in [Case::UpdateReadError, Case::UpdateWriteError] {
        exercise(false, case).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn login_not_eligible_still_persists_exhausted() {
    exercise(true, Case::Refused).await;
}

#[tokio::test(flavor = "current_thread")]
async fn import_not_eligible_still_persists_exhausted() {
    exercise(false, Case::Refused).await;
}

#[tokio::test(flavor = "current_thread")]
async fn login_success_still_persists_admitted_project() {
    exercise(true, Case::Success).await;
}

#[tokio::test(flavor = "current_thread")]
async fn import_success_still_persists_admitted_project() {
    exercise(false, Case::Success).await;
}
