//! Inert login/import coverage for the admitted primary team/project pair.
use super::*;

const NEXT_TEAM: &str = "team-next";
const COOKIE: &str = "session=fixture-only";

struct AdmissionPorts {
    inner: Arc<FixturePorts>,
    switched: bool,
    create: bool,
    calls: Mutex<Vec<String>>,
    writes: Mutex<Vec<ManagedAccount>>,
}

fn credit(available: f64) -> CreditSnapshot {
    CreditSnapshot {
        available,
        funds: 1,
        grants: 1,
        allocated: available,
    }
}

impl AdmissionPorts {
    fn record(&self, call: &str) {
        self.calls.lock().unwrap().push(call.into());
    }

    fn assert_session(&self, session: &Session) {
        assert_eq!(session.active_user.0, "user-2");
        assert_eq!(session.cookie_header, COOKIE);
    }

    fn identity(&self) -> Value {
        let spaces: Vec<_> = self
            .inner
            .workspaces
            .lock()
            .unwrap()
            .iter()
            .map(|(team, _)| json!({"scope": "organization", "ownerId": team}))
            .collect();
        json!({"id": "user-2", "spaces": spaces})
    }

    fn project_call(&self, session: &Session, team: &TeamId, method: &str) {
        self.assert_session(session);
        assert!(self.switched, "same-team project must be reused");
        assert_eq!(team.0, NEXT_TEAM);
        let records = self.inner.records.lock().unwrap();
        assert_eq!(records[1].current_team.as_ref(), Some(team));
        assert_eq!(records[1].current_project, None);
        assert!(records[1].all_scopes().is_empty());
        self.record(method);
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
impl FramerClient for AdmissionPorts {
    async fn me(&self, session: &Session) -> Result<Value, String> {
        assert_eq!(session.cookie_header, COOKIE);
        if session.active_user.0.is_empty() {
            self.record("probe");
        } else {
            self.assert_session(session);
            self.record("me");
        }
        Ok(self.identity())
    }

    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        self.assert_session(session);
        self.record(&format!("usage:{}", team.0));
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
                ProjectId("older-selected-project".into()),
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

fn admission_fixture(
    login: bool,
    switched: bool,
    create: bool,
) -> (AppState, Arc<AdmissionPorts>, ManagedAccount) {
    let (_, inner) = fixture(true);
    *inner.workspaces.lock().unwrap() = vec![
        ("team-2", if switched { 0.0 } else { 100.0 }),
        (NEXT_TEAM, if switched { 100.0 } else { 0.0 }),
    ];
    let mut initial = account(2);
    initial.credit = Some(credit(7.0));
    initial.consecutive_failures = 3;
    initial.cooldown_step = 2;
    *inner.records.lock().unwrap() = vec![account(1), initial.clone()];
    let ports = Arc::new(AdmissionPorts {
        inner,
        switched,
        create,
        calls: Mutex::new(Vec::new()),
        writes: Mutex::new(Vec::new()),
    });
    let state = AppState::fixture(
        ports.clone(),
        ports.clone(),
        ports.clone(),
        ports.clone(),
        policy(if login { &[2] } else { &[1] }),
        framer_core::gate::ConcurrencyGate::new(),
    );
    (state, ports, initial)
}

async fn assert_admission(login: bool, switched: bool, create: bool) {
    let (state, ports, initial) = admission_fixture(login, switched, create);
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

fn assert_saved(ports: &AdmissionPorts, initial: &ManagedAccount, response: &Value, login: bool) {
    let mut admitted = initial.clone();
    admitted.credit = Some(credit(100.0));
    if ports.switched {
        admitted.current_team = Some(TeamId(NEXT_TEAM.into()));
        admitted.current_project = None;
    }
    let mut expected = admitted.clone();
    if ports.switched {
        expected.current_project = Some(ProjectId(
            if ports.create {
                "created-selected-project"
            } else {
                "newest-selected-project"
            }
            .into(),
        ));
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
        json!([
            {"team": expected.current_team, "project": expected.current_project}
        ])
    );
    drop(records);
    let mut writes = vec![initial.clone(), admitted, expected.clone()];
    if !login && ports.switched && ports.create {
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

fn assert_calls(ports: &AdmissionPorts, login: bool) {
    let mut expected = if login {
        vec!["auth_complete", "save", "grant", "upsert", "me", "upsert"]
    } else {
        vec!["probe", "save", "grant", "proxy", "upsert", "me", "upsert"]
    };
    if ports.switched {
        expected.push("team_projects");
        if ports.create {
            expected.push("create_project");
            if !login {
                expected.push("upsert");
            }
        }
    }
    expected.extend(["me", "upsert"]);
    let calls = ports.calls.lock().unwrap();
    let non_usage: Vec<_> = calls
        .iter()
        .map(String::as_str)
        .filter(|call| !call.starts_with("usage:"))
        .collect();
    assert_eq!(non_usage, expected);
    // Credit probes are concurrent; their relative polling order is irrelevant.
    let mut usage: Vec<_> = calls
        .iter()
        .map(String::as_str)
        .filter(|call| call.starts_with("usage:"))
        .collect();
    usage.sort_unstable();
    assert_eq!(
        usage,
        vec![
            "usage:team-2",
            "usage:team-2",
            "usage:team-next",
            "usage:team-next"
        ]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn login_team_switch_resolves_project_in_selected_team() {
    for create in [false, true] {
        assert_admission(true, true, create).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn import_team_switch_resolves_project_in_selected_team() {
    for create in [false, true] {
        assert_admission(false, true, create).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn same_team_login_reuses_existing_project() {
    assert_admission(true, false, false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn same_team_import_reuses_existing_project() {
    assert_admission(false, false, false).await;
}
