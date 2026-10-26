//! Inert coverage for workspace claims shared by registration and provisioning.
use super::*;
use framer_core::account::{AccountState, WorkspaceScope};
use std::collections::BTreeSet;

const PRIMARY: &str = "subject-primary";
const OWN_EXTRA: &str = "subject-extra";
const PEER_PRIMARY: &str = "peer-primary";
const PEER_EXTRA: &str = "peer-extra";
const FREE_EXTRA: &str = "unclaimed-extra";
const PRIMARY_PROJECT: &str = "subject-project";

struct ClaimPorts {
    inner: Arc<FixturePorts>,
    create: bool,
    auth_calls: AtomicUsize,
    grants: AtomicUsize,
    project_calls: Mutex<Vec<(&'static str, String)>>,
}

fn scope(team: &str, project: &str) -> WorkspaceScope {
    WorkspaceScope {
        team: TeamId(team.into()),
        project: ProjectId(project.into()),
    }
}

impl ClaimPorts {
    fn assert_session(&self, session: &Session) {
        assert_eq!(session.active_user.0, "user-2");
        assert_eq!(session.cookie_header, "session=fixture-only");
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

    fn project_call(&self, session: &Session, team: &TeamId, method: &'static str) {
        self.assert_session(session);
        self.project_calls
            .lock()
            .unwrap()
            .push((method, team.0.clone()));
    }
}

fn resolved_project(team: &str, created: bool) -> ProjectId {
    let prefix = if created { "created" } else { "adopted" };
    ProjectId(format!("{prefix}:{team}"))
}

#[async_trait::async_trait]
impl FramerClient for ClaimPorts {
    async fn me(&self, session: &Session) -> Result<Value, String> {
        if !session.active_user.0.is_empty() {
            self.assert_session(session);
        }
        self.inner.me(session).await
    }

    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        self.assert_session(session);
        self.inner.usage(session, team).await
    }

    async fn claim_workspace_grant(&self, session: &Session) -> Result<bool, String> {
        self.assert_session(session);
        assert_eq!(self.inner.saved.lock().unwrap().len(), 1);
        self.grants.fetch_add(1, Ordering::SeqCst);
        Ok(false)
    }

    async fn team_projects(
        &self,
        session: &Session,
        team: &TeamId,
    ) -> Result<Vec<ProjectId>, String> {
        self.project_call(session, team, "list");
        Ok(if self.create {
            Vec::new()
        } else {
            vec![
                resolved_project(&team.0, false),
                ProjectId("older-project".into()),
            ]
        })
    }

    async fn create_project(&self, session: &Session, team: &TeamId) -> Result<ProjectId, String> {
        self.project_call(session, team, "create");
        assert!(self.create, "an existing project must be adopted");
        Ok(resolved_project(&team.0, true))
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

#[async_trait::async_trait]
impl BrowserBridge for ClaimPorts {
    async fn auth_complete(&self, id: &AccountId, token: &str) -> Result<(String, Value), String> {
        assert_eq!(*id, account(2).id);
        assert_eq!(token, "fixture-token");
        self.auth_calls.fetch_add(1, Ordering::SeqCst);
        Ok(("session=fixture-only".into(), self.identity()))
    }

    async fn register_account_proxy(
        &self,
        id: &AccountId,
        proxy: Option<String>,
    ) -> Result<(), String> {
        assert_eq!(*id, account(2).id);
        assert_eq!(proxy, account(2).proxy);
        self.inner.register_account_proxy(id, proxy).await
    }

    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        self.inner.unexpected("browser");
    }

    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        self.inner.unexpected("browser");
    }
}

#[derive(Clone, Copy)]
enum Scenario {
    Refused,
    Filter,
    Available,
}

fn initial_records() -> Vec<ManagedAccount> {
    let mut peer = account(1);
    peer.current_team = Some(TeamId(PEER_PRIMARY.into()));
    peer.current_project = None;
    peer.extra_workspaces = vec![scope(PEER_EXTRA, "peer-extra-project")];
    let mut subject = account(2);
    subject.current_team = Some(TeamId(PRIMARY.into()));
    subject.current_project = Some(ProjectId(PRIMARY_PROJECT.into()));
    subject.extra_workspaces = vec![scope(OWN_EXTRA, "old-own-extra-project")];
    subject.consecutive_failures = 3;
    subject.cooldown_step = 2;
    vec![peer, subject]
}

fn workspaces(scenario: Scenario) -> Vec<(&'static str, f64)> {
    match scenario {
        Scenario::Refused => vec![(PEER_EXTRA, 100.0)],
        Scenario::Filter => vec![
            (PRIMARY, 300.0),
            (PEER_PRIMARY, 250.0),
            (PEER_EXTRA, 200.0),
            (OWN_EXTRA, 150.0),
            (FREE_EXTRA, 100.0),
        ],
        Scenario::Available => vec![(PRIMARY, 300.0), (OWN_EXTRA, 150.0), (FREE_EXTRA, 100.0)],
    }
}

fn claims_fixture(login: bool, scenario: Scenario, create: bool) -> (AppState, Arc<ClaimPorts>) {
    let (_, inner) = fixture(true);
    *inner.records.lock().unwrap() = initial_records();
    *inner.workspaces.lock().unwrap() = workspaces(scenario);
    let ports = Arc::new(ClaimPorts {
        inner,
        create,
        auth_calls: AtomicUsize::new(0),
        grants: AtomicUsize::new(0),
        project_calls: Mutex::new(Vec::new()),
    });
    let state = AppState::fixture(
        ports.clone(),
        ports.clone(),
        ports.inner.clone(),
        ports.inner.clone(),
        policy(if login { &[2] } else { &[1] }),
        framer_core::gate::ConcurrencyGate::new(),
    );
    (state, ports)
}

async fn register(
    login: bool,
    state: AppState,
) -> crate::pool_control::CleanupOutcome<Json<Value>> {
    // Explicit project input keeps these cases independent of metadata-project
    // clearing. Imports are excluded from prewarm by their fixture policy.
    let mut body = json!({"account_id": account(2).id, "project_id": PRIMARY_PROJECT});
    if login {
        body["email_token"] = json!("fixture-token");
        crate::complete_login_observed(State(state), Json(serde_json::from_value(body).unwrap()))
            .await
    } else {
        body["session_cookie"] = json!("fixture-only");
        body["expected_framer_user_id"] = json!("user-2");
        crate::import_account_observed(State(state), Json(serde_json::from_value(body).unwrap()))
            .await
    }
}

fn assert_records(ports: &ClaimPorts, subject: ManagedAccount) {
    let expected = vec![initial_records().remove(0), subject];
    assert_eq!(
        serde_json::to_value(&*ports.inner.records.lock().unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

fn assert_refused(ports: &ClaimPorts, result: Result<Json<Value>, ApiError>) {
    assert!(matches!(result, Err(ApiError::Internal(ref message))
        if message.contains("every credited workspace is already in use")
            && message.contains(PEER_EXTRA)));
    let mut expected = initial_records().remove(1);
    expected.current_team = Some(TeamId(PEER_EXTRA.into()));
    expected.state = AccountState::Exhausted;
    assert_records(ports, expected);
    assert!(ports.project_calls.lock().unwrap().is_empty());
    assert_eq!(ports.inner.state_writes.load(Ordering::SeqCst), 2);
}

fn assert_provisioned(ports: &ClaimPorts, response: &Value) {
    let mut expected = initial_records().remove(1);
    expected.credit = Some(CreditSnapshot {
        available: 300.0,
        allocated: 300.0,
        funds: 1,
        grants: 1,
    });
    expected.extra_workspaces = [OWN_EXTRA, FREE_EXTRA]
        .into_iter()
        .map(|team| WorkspaceScope {
            team: TeamId(team.into()),
            project: resolved_project(team, ports.create),
        })
        .collect();
    assert_eq!(response["team"], PRIMARY);
    assert_eq!(response["project"], PRIMARY_PROJECT);
    assert_records(ports, expected);
    let teams: Vec<_> = ports.inner.records.lock().unwrap()[1]
        .all_scopes()
        .into_iter()
        .map(|scope| scope.team.0)
        .collect();
    assert_eq!(teams, [PRIMARY, OWN_EXTRA, FREE_EXTRA]);
    let mut calls = Vec::new();
    for team in [OWN_EXTRA, FREE_EXTRA] {
        calls.push(("list", team.to_string()));
        if ports.create {
            calls.push(("create", team.to_string()));
        }
    }
    assert_eq!(*ports.project_calls.lock().unwrap(), calls);
    assert_eq!(ports.inner.state_writes.load(Ordering::SeqCst), 3);
}

async fn exercise(login: bool, scenario: Scenario, create: bool) {
    let (state, ports) = claims_fixture(login, scenario, create);
    let serving = state.serving_accounts.clone();
    let config = serving.config();
    let outcome = register(login, state).await;
    assert!(outcome.cleanup_confirmed);
    match scenario {
        Scenario::Refused => assert_refused(&ports, outcome.result),
        _ => {
            let response = outcome.result.unwrap().0;
            assert_eq!(
                response["status"],
                if login { "signed-in" } else { "imported" }
            );
            if login {
                assert_eq!(response["credits"], 300.0);
            } else {
                assert_eq!(response["serving_allowed"], false);
            }
            assert_provisioned(&ports, &response);
        }
    }
    assert_eq!(serving.config(), config);
    assert!(serving.mutations.lock().unwrap().is_empty());
    assert!(serving.registration.try_lock().is_ok());
    assert_eq!(
        Arc::strong_count(&ports),
        1,
        "background task retained the ports"
    );
    assert_eq!(ports.auth_calls.load(Ordering::SeqCst), usize::from(login));
    assert_eq!(ports.grants.load(Ordering::SeqCst), 1);
    assert_eq!(ports.inner.session_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.inner.browser_calls.load(Ordering::SeqCst), 0);
    let saved = ports.inner.saved.lock().unwrap();
    assert_eq!(saved.len(), 1);
    ports.assert_session(&saved[0]);
    let proxies = if login {
        Vec::new()
    } else {
        vec![(account(2).id, account(2).proxy)]
    };
    assert_eq!(*ports.inner.registered_proxies.lock().unwrap(), proxies);
    let scans = if matches!(scenario, Scenario::Refused) {
        1
    } else {
        2
    };
    assert_eq!(
        ports.inner.client_calls.load(Ordering::SeqCst),
        scans * (1 + workspaces(scenario).len()) + usize::from(!login)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn login_refuses_a_peer_secondary_workspace() {
    exercise(true, Scenario::Refused, false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn import_refuses_a_peer_secondary_workspace() {
    exercise(false, Scenario::Refused, false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn login_skips_peer_workspaces_during_extra_provisioning() {
    for create in [false, true] {
        exercise(true, Scenario::Filter, create).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn import_skips_peer_workspaces_during_extra_provisioning() {
    for create in [false, true] {
        exercise(false, Scenario::Filter, create).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn login_preserves_unclaimed_and_own_workspace_scopes() {
    for create in [false, true] {
        exercise(true, Scenario::Available, create).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn import_preserves_unclaimed_and_own_workspace_scopes() {
    for create in [false, true] {
        exercise(false, Scenario::Available, create).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn claims_include_full_peer_ids_without_requiring_a_primary_project() {
    let (state, ports) = fixture(false);
    let mut records = initial_records();
    records[0].current_team = Some(TeamId("shared-prefix-primary".into()));
    records[0].extra_workspaces = vec![
        scope("shared-prefix-extra-a", "project-a"),
        scope("shared-prefix-extra-b", "project-b"),
        scope("équipe-extra", "project-c"),
    ];
    let mut extra_only = account(3);
    extra_only.current_team = None;
    extra_only.current_project = None;
    extra_only.extra_workspaces = vec![scope("extra-only", "project-d")];
    records.push(extra_only);
    let before = serde_json::to_value(&records).unwrap();
    *ports.records.lock().unwrap() = records;
    let claims = crate::claimed_workspaces(&state, account(2).id)
        .await
        .unwrap();
    let expected: BTreeSet<String> = [
        "shared-prefix-primary",
        "shared-prefix-extra-a",
        "shared-prefix-extra-b",
        "équipe-extra",
        "extra-only",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(claims, expected);
    assert!(!claims.contains(PRIMARY));
    assert!(!claims.contains(OWN_EXTRA));
    assert_eq!(
        serde_json::to_value(&*ports.records.lock().unwrap()).unwrap(),
        before
    );
    ports.assert_rejected_before_io(1);
}
