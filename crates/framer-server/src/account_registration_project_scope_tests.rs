//! Inert coverage for metadata-driven team changes before credit admission.
use super::*;

const OLD_TEAM: &str = "team-2";
const NEXT_TEAM: &str = "team-next";
const EXPLICIT_PROJECT: &str = "explicit-project";

struct MetadataPorts {
    inner: Arc<FixturePorts>,
    metadata_team: Option<&'static str>,
    explicit: bool,
    create: bool,
    calls: Mutex<Vec<String>>,
    writes: Mutex<Vec<ManagedAccount>>,
}

impl MetadataPorts {
    fn record(&self, name: &str) {
        self.calls.lock().unwrap().push(name.into());
    }

    fn team(&self) -> &'static str {
        self.metadata_team.unwrap_or(OLD_TEAM)
    }

    fn needs_project(&self) -> bool {
        self.metadata_team == Some(NEXT_TEAM) && !self.explicit
    }

    fn identity(&self) -> Value {
        // Initial identity metadata may omit the workspace. A later credit
        // scan returns the full organization list; personal spaces have no ledger.
        let scope = if self.metadata_team.is_some() {
            "organization"
        } else {
            "user"
        };
        json!({"id": "user-2", "spaces": [{"scope": scope, "ownerId": self.team()}]})
    }

    fn assert_session(&self, session: &Session) {
        assert_eq!(session.active_user.0, "user-2");
        assert_eq!(session.cookie_header, "session=fixture-only");
    }

    fn project_call(&self, session: &Session, team: &TeamId, method: &str) {
        self.assert_session(session);
        assert!(
            self.needs_project(),
            "existing or explicit project must be reused"
        );
        assert_eq!(team.0, NEXT_TEAM);
        let records = self.inner.records.lock().unwrap();
        assert_eq!(records[1].current_team.as_ref(), Some(team));
        assert_eq!(records[1].current_project, None);
        assert!(records[1].all_scopes().is_empty());
        self.record(method);
    }
}

#[async_trait::async_trait]
impl BrowserBridge for MetadataPorts {
    async fn auth_complete(&self, id: &AccountId, token: &str) -> Result<(String, Value), String> {
        assert_eq!(*id, account(2).id);
        assert_eq!(token, "fixture-token");
        self.record("auth_complete");
        Ok(("session=fixture-only".into(), self.identity()))
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
impl StateStore for MetadataPorts {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        self.inner.get(id).await
    }

    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        self.inner.list().await
    }

    async fn upsert(&self, current: &ManagedAccount) -> Result<(), String> {
        self.inner.upsert(current).await?;
        self.record("upsert");
        self.writes.lock().unwrap().push(current.clone());
        Ok(())
    }

    async fn remove(&self, _: AccountId) -> Result<(), String> {
        self.inner.unexpected("state");
    }
}

#[async_trait::async_trait]
impl FramerClient for MetadataPorts {
    async fn me(&self, session: &Session) -> Result<Value, String> {
        assert_eq!(session.cookie_header, "session=fixture-only");
        if session.active_user.0.is_empty() {
            self.record("probe");
        } else {
            self.assert_session(session);
            self.record("me");
            return Ok(json!({"id": "user-2", "spaces": [{
                "scope": "organization", "ownerId": self.team()
            }]}));
        }
        Ok(self.identity())
    }

    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        self.assert_session(session);
        assert_eq!(team.0, self.team());
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
        self.project_call(session, team, "team_projects");
        Ok(if self.create {
            Vec::new()
        } else {
            vec![
                ProjectId("newest-selected-project".into()),
                ProjectId("older-project".into()),
            ]
        })
    }

    async fn create_project(&self, session: &Session, team: &TeamId) -> Result<ProjectId, String> {
        self.project_call(session, team, "create_project");
        assert!(self.create, "an existing project must be adopted");
        Ok(ProjectId("created-selected-project".into()))
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

fn metadata_fixture(
    login: bool,
    metadata_team: Option<&'static str>,
    explicit: bool,
    create: bool,
) -> (AppState, Arc<MetadataPorts>, ManagedAccount) {
    let (_, inner) = fixture(true);
    *inner.workspaces.lock().unwrap() = vec![(metadata_team.unwrap_or(OLD_TEAM), 100.0)];
    let mut initial = account(2);
    initial.credit = Some(credit(7.0));
    initial.consecutive_failures = 3;
    initial.cooldown_step = 2;
    *inner.records.lock().unwrap() = vec![account(1), initial.clone()];
    let ports = Arc::new(MetadataPorts {
        inner,
        metadata_team,
        explicit,
        create,
        calls: Mutex::new(Vec::new()),
        writes: Mutex::new(Vec::new()),
    });
    let state = AppState::fixture(
        ports.clone(),
        ports.clone(),
        ports.inner.clone(),
        ports.clone(),
        policy(if login { &[2] } else { &[1] }),
        framer_core::gate::ConcurrencyGate::new(),
    );
    (state, ports, initial)
}

async fn exercise(login: bool, metadata_team: Option<&'static str>, explicit: bool, create: bool) {
    let (state, ports, initial) = metadata_fixture(login, metadata_team, explicit, create);
    let serving = state.serving_accounts.clone();
    let config = serving.config();
    let mut body = json!({
        "account_id": initial.id,
        "project_id": explicit.then_some(EXPLICIT_PROJECT)
    });
    let outcome = if login {
        body["email_token"] = json!("fixture-token");
        crate::complete_login_observed(State(state), Json(serde_json::from_value(body).unwrap()))
            .await
    } else {
        body["session_cookie"] = json!("fixture-only");
        body["expected_framer_user_id"] = json!("user-2");
        crate::import_account_observed(State(state), Json(serde_json::from_value(body).unwrap()))
            .await
    };
    assert!(outcome.cleanup_confirmed);
    let response = outcome.result.unwrap().0;
    assert_eq!(
        response["status"],
        if login { "signed-in" } else { "imported" }
    );
    if !login {
        assert_eq!(response["serving_allowed"], false);
    }
    assert_saved(&ports, &initial, &response, login);
    assert_calls(&ports, login);
    assert_eq!(serving.config(), config);
    assert!(serving.mutations.lock().unwrap().is_empty());
    assert_eq!(
        Arc::strong_count(&ports),
        1,
        "background work retained ports"
    );
}

fn assert_saved(ports: &MetadataPorts, initial: &ManagedAccount, response: &Value, login: bool) {
    let mut registered = initial.clone();
    registered.current_team = Some(TeamId(ports.team().into()));
    registered.current_project = if ports.explicit {
        Some(ProjectId(EXPLICIT_PROJECT.into()))
    } else if ports.needs_project() {
        None
    } else {
        initial.current_project.clone()
    };
    let mut admitted = registered.clone();
    admitted.credit = Some(credit(100.0));
    let mut expected = admitted.clone();
    if ports.needs_project() {
        let project = if ports.create {
            "created-selected-project"
        } else {
            "newest-selected-project"
        };
        expected.current_project = Some(ProjectId(project.into()));
    }
    assert_eq!(response["team"], json!(expected.current_team));
    assert_eq!(response["project"], json!(expected.current_project));
    assert_eq!(response["account_id"], json!(initial.id));
    assert_eq!(response["framer_user_id"], json!(initial.framer_user_id));
    let records = ports.inner.records.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&*records).unwrap(),
        serde_json::to_value([account(1), expected.clone()]).unwrap()
    );
    assert_eq!(
        serde_json::to_value(records[1].all_scopes()).unwrap(),
        json!([{"team": expected.current_team, "project": expected.current_project}])
    );
    drop(records);
    let mut writes = vec![registered, admitted, expected.clone()];
    if !login && ports.needs_project() && ports.create {
        writes.push(expected);
    }
    assert_eq!(
        serde_json::to_value(&*ports.writes.lock().unwrap()).unwrap(),
        serde_json::to_value(&writes).unwrap()
    );
    assert_eq!(
        ports.inner.state_writes.load(Ordering::SeqCst),
        writes.len()
    );
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

fn assert_calls(ports: &MetadataPorts, login: bool) {
    let mut expected = vec![if login { "auth_complete" } else { "probe" }, "grant"];
    if !login {
        expected.push("proxy");
    }
    expected.extend(["upsert", "me", "usage", "upsert"]);
    if ports.needs_project() {
        expected.push("team_projects");
        if ports.create {
            expected.push("create_project");
            if !login {
                expected.push("upsert");
            }
        }
    }
    expected.extend(["me", "usage", "upsert"]);
    let calls = ports.calls.lock().unwrap();
    assert_eq!(
        calls.iter().map(String::as_str).collect::<Vec<_>>(),
        expected
    );
}

#[tokio::test(flavor = "current_thread")]
async fn login_metadata_team_change_resolves_project() {
    for create in [false, true] {
        exercise(true, Some(NEXT_TEAM), false, create).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn import_metadata_team_change_resolves_project() {
    for create in [false, true] {
        exercise(false, Some(NEXT_TEAM), false, create).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn login_reuses_project_when_metadata_keeps_or_omits_team() {
    for metadata in [Some(OLD_TEAM), None] {
        exercise(true, metadata, false, false).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn import_reuses_project_when_metadata_keeps_or_omits_team() {
    for metadata in [Some(OLD_TEAM), None] {
        exercise(false, metadata, false, false).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn login_explicit_project_overrides_prior_scope() {
    for metadata in [Some(NEXT_TEAM), Some(OLD_TEAM), None] {
        exercise(true, metadata, true, false).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn import_explicit_project_overrides_prior_scope() {
    for metadata in [Some(NEXT_TEAM), Some(OLD_TEAM), None] {
        exercise(false, metadata, true, false).await;
    }
}
