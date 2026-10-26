//! Provisioning fixtures exercise metadata and project retention without I/O.
use super::*;

struct ProvisioningClient {
    team: TeamId,
    existing: bool,
    calls: Mutex<Vec<&'static str>>,
}

impl ProvisioningClient {
    fn record(&self, session: &Session, method: &'static str) {
        assert_eq!(session.active_user.0, "user-1");
        assert_eq!(session.cookie_header, "session=fixture-only");
        self.calls.lock().unwrap().push(method);
    }
}

#[async_trait::async_trait]
impl FramerClient for ProvisioningClient {
    async fn me(&self, session: &Session) -> Result<Value, String> {
        self.record(session, "me");
        Ok(json!({"id": "user-1", "spaces": [
            {"scope": "organization", "ownerId": self.team.0}
        ]}))
    }

    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        self.record(session, "usage");
        assert_eq!(team, &self.team);
        Ok(CreditSnapshot {
            available: 100.0,
            funds: 1,
            grants: 1,
            allocated: 100.0,
        })
    }

    async fn team_projects(
        &self,
        session: &Session,
        team: &TeamId,
    ) -> Result<Vec<ProjectId>, String> {
        self.record(session, "team_projects");
        assert_eq!(team, &self.team);
        Ok(if self.existing {
            vec![
                ProjectId("newest-existing-project".into()),
                ProjectId("older-existing-project".into()),
            ]
        } else {
            Vec::new()
        })
    }

    async fn create_project(&self, session: &Session, team: &TeamId) -> Result<ProjectId, String> {
        self.record(session, "create_project");
        assert_eq!(team, &self.team);
        assert!(!self.existing);
        Ok(ProjectId("created-extra-project".into()))
    }

    async fn request_magic_link(&self, _email: &str, _castle: &CastleToken) -> Result<(), String> {
        panic!("unexpected magic-link request");
    }

    async fn complete_magic_link(
        &self,
        _token: &str,
        _castle: &CastleToken,
    ) -> Result<Session, String> {
        panic!("unexpected magic-link completion");
    }

    async fn claim_workspace_grant(&self, _session: &Session) -> Result<bool, String> {
        panic!("unexpected workspace grant");
    }

    async fn create_workspace(
        &self,
        _session: &Session,
        _spec: &WorkspaceSpec,
    ) -> Result<TeamId, String> {
        panic!("unexpected workspace creation");
    }

    async fn run_inference(
        &self,
        _session: &Session,
        _browser: &dyn BrowserBridge,
        _account: &AccountId,
        _project: &ProjectId,
        _request: &InferenceRequest,
    ) -> Result<InferenceResult, String> {
        panic!("unexpected inference");
    }

    async fn proxy(
        &self,
        _session: &Session,
        _browser: &dyn BrowserBridge,
        _account: &AccountId,
        _project: &ProjectId,
        _dialect: &str,
        _payload: Value,
        _chat_session: &str,
    ) -> Result<ProxyResponse, String> {
        panic!("unexpected proxy call");
    }

    async fn freechat_long(
        &self,
        _session: &Session,
        _browser: &dyn BrowserBridge,
        _account: &AccountId,
        _project: &ProjectId,
        _payload: Value,
        _cfg: &ChainConfig,
        _chat_session: &str,
        _events: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
    ) -> Result<AssembledResponse, String> {
        panic!("unexpected freechat chain");
    }

    async fn native_long(
        &self,
        _session: &Session,
        _browser: &dyn BrowserBridge,
        _account: &AccountId,
        _project: &ProjectId,
        _payload: Value,
        _cfg: &ChainConfig,
        _chat_session: &str,
        _events: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
    ) -> Result<AssembledResponses, String> {
        panic!("unexpected native chain");
    }
}

async fn assert_extra_scope(team: &str, existing: bool) {
    let (mut state, ports) = fixture(false);
    let client = Arc::new(ProvisioningClient {
        team: TeamId(team.into()),
        existing,
        calls: Mutex::new(Vec::new()),
    });
    state.runtime_mut().expect("unique fixture runtime").client = client.clone();
    let mut current = account(1);
    let mut expected = current.clone();
    let project = if existing {
        "newest-existing-project"
    } else {
        "created-extra-project"
    };
    expected.extra_workspaces = vec![framer_core::account::WorkspaceScope {
        team: TeamId(team.into()),
        project: ProjectId(project.into()),
    }];
    let before = serde_json::to_value(&*ports.records.lock().unwrap()).unwrap();
    let config = state.serving_accounts.config();
    let session = Session {
        active_user: current.framer_user_id.clone(),
        cookie_header: "session=fixture-only".into(),
    };
    let cleanup = crate::pool_control::CleanupEvidence::new();
    crate::populate_extra_scopes(&state, &session, &mut current, &cleanup).await;
    assert!(cleanup.confirmed());
    assert_eq!(
        serde_json::to_value(&current).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    let mut calls = vec!["me", "usage", "team_projects"];
    if !existing {
        calls.push("create_project");
    }
    assert_eq!(*client.calls.lock().unwrap(), calls);
    ports.assert_rejected_before_io(0);
    assert_eq!(
        serde_json::to_value(&*ports.records.lock().unwrap()).unwrap(),
        before
    );
    assert_eq!(state.serving_accounts.config(), config);
    assert!(ports.saved.lock().unwrap().is_empty());
    assert!(ports.registered_proxies.lock().unwrap().is_empty());
    assert!(state.serving_accounts.mutations.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn short_metadata_team_ids_retain_created_project() {
    for team in ["", "x", "short", "1234567"] {
        assert_extra_scope(team, false).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn ascii_team_id_preserves_creation_and_identity() {
    assert_extra_scope("12345678-ascii-team", false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn existing_project_is_adopted_without_creation() {
    for team in ["x", "1234567é-tail", "12345678-ascii-team"] {
        assert_extra_scope(team, true).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn utf8_metadata_team_ids_retain_created_project() {
    for team in ["1234567é-tail", "abc🚀é-tail", "界界界", "ééééé", "🚀🚀🚀"] {
        assert_extra_scope(team, false).await;
    }
}
