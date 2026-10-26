//! Inert caller coverage for incomplete extra-scope scans after admission.
use super::*;
use framer_core::account::WorkspaceScope;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SecondScan { Failed, Partial, Complete }

struct ScanPorts {
    inner: Arc<FixturePorts>,
    second: SecondScan,
    scans: AtomicUsize,
    writes: Mutex<Vec<ManagedAccount>>,
    calls: Mutex<Vec<&'static str>>,
}

impl ScanPorts {
    fn record(&self, call: &'static str) { self.calls.lock().unwrap().push(call); }

    fn identity(&self) -> Value {
        json!({"id": "user-2", "spaces": [
            {"scope": "organization", "ownerId": "team-2"},
            {"scope": "organization", "ownerId": "new-extra"}
        ]})
    }

    fn check_session(&self, session: &Session) {
        assert_eq!(session.active_user.0, "user-2");
        assert_eq!(session.cookie_header, "session=fixture-only");
    }
}

#[async_trait::async_trait]
impl BrowserBridge for ScanPorts {
    async fn auth_complete(&self, id: &AccountId, token: &str) -> Result<(String, Value), String> {
        assert_eq!(*id, account(2).id);
        assert_eq!(token, "fixture-token");
        self.record("auth_complete");
        Ok(("session=fixture-only".into(), self.identity()))
    }
    async fn register_account_proxy(&self, id: &AccountId, proxy: Option<String>) -> Result<(), String> {
        self.record("proxy");
        self.inner.register_account_proxy(id, proxy).await
    }
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> { self.inner.unexpected("browser"); }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> { self.inner.unexpected("browser"); }
}

#[async_trait::async_trait]
impl StateStore for ScanPorts {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> { self.inner.get(id).await }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> { self.inner.list().await }
    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String> {
        self.record("upsert");
        self.writes.lock().unwrap().push(account.clone());
        self.inner.upsert(account).await
    }
    async fn remove(&self, _: AccountId) -> Result<(), String> { self.inner.unexpected("state"); }
}

#[async_trait::async_trait]
impl FramerClient for ScanPorts {
    async fn me(&self, session: &Session) -> Result<Value, String> {
        assert_eq!(session.cookie_header, "session=fixture-only");
        if session.active_user.0.is_empty() {
            self.record("probe");
        } else {
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
        if self.scans.load(Ordering::SeqCst) == 2 && self.second != SecondScan::Complete {
            if team.0 == "team-2" || self.second == SecondScan::Failed {
                return Err("fixture second scan failed".into());
            }
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
    ) -> Result<InferenceResult, String> { self.inner.unexpected("client"); }

    async fn proxy(
        &self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: &str, _: Value, _: &str,
    ) -> Result<ProxyResponse, String> { self.inner.unexpected("client"); }

    async fn freechat_long(
        &self, _: &Session, _: &dyn BrowserBridge, _: &AccountId, _: &ProjectId,
        _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
    ) -> Result<AssembledResponse, String> { self.inner.unexpected("client"); }

    async fn native_long(
        &self, _: &Session, _: &dyn BrowserBridge, _: &AccountId, _: &ProjectId,
        _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
    ) -> Result<AssembledResponses, String> { self.inner.unexpected("client"); }
}

fn credit(available: f64) -> CreditSnapshot {
    CreditSnapshot { available, funds: 1, grants: 1, allocated: available }
}

fn scan_fixture(login: bool, second: SecondScan) -> (AppState, Arc<ScanPorts>, ManagedAccount) {
    let (_, inner) = fixture(true);
    let mut initial = account(2);
    initial.credit = Some(credit(7.0));
    initial.consecutive_failures = 3;
    initial.cooldown_step = 2;
    initial.cooling_until_ms = Some(12_345);
    initial.extra_workspaces = vec![WorkspaceScope {
        team: TeamId("saved-extra".into()), project: ProjectId("saved-extra-project".into()),
    }];
    *inner.records.lock().unwrap() = vec![account(1), initial.clone()];
    let ports = Arc::new(ScanPorts {
        inner, second, scans: AtomicUsize::new(0),
        writes: Mutex::new(Vec::new()), calls: Mutex::new(Vec::new()),
    });
    let state = AppState::fixture(
        ports.clone(), ports.clone(), ports.inner.clone(), ports.clone(),
        policy(if login { &[2] } else { &[1] }),
        framer_core::gate::ConcurrencyGate::new(),
    );
    (state, ports, initial)
}

async fn exercise(login: bool, second: SecondScan) {
    let (state, ports, initial) = scan_fixture(login, second);
    let serving = state.serving_accounts.clone();
    let config = serving.config();
    let outcome = if login {
        let body = serde_json::from_value(json!({
            "account_id": initial.id, "email_token": "fixture-token"
        })).unwrap();
        crate::complete_login_observed(State(state), Json(body)).await
    } else {
        let body = serde_json::from_value(json!({
            "account_id": initial.id, "session_cookie": "session=fixture-only",
            "expected_framer_user_id": "user-2"
        })).unwrap();
        crate::import_account_observed(State(state), Json(body)).await
    };
    assert!(outcome.cleanup_confirmed);
    let response = outcome.result.unwrap().0;
    assert_eq!(response["status"], if login { "signed-in" } else { "imported" });
    assert_eq!(response["team"], "team-2");
    assert_eq!(response["project"], "project-2");
    if !login { assert_eq!(response["serving_allowed"], false); }
    assert_saved(&ports, &initial, login);
    assert_calls(&ports, login);
    assert_eq!(serving.config(), config);
    assert!(serving.mutations.lock().unwrap().is_empty());
    assert!(serving.registration.try_lock().is_ok());
    assert!(serving.health_update.try_lock().is_ok());
    assert!(serving.residents.snapshot().unwrap().residents.iter().all(|slot| {
        slot.users == 0 && !slot.uncertain && slot.replacement.is_none()
    }));
    assert_eq!(Arc::strong_count(&ports), 1, "background work retained ports");
}

fn assert_saved(ports: &ScanPorts, initial: &ManagedAccount, login: bool) {
    let mut admitted = initial.clone();
    admitted.credit = Some(credit(100.0));
    let mut expected = admitted.clone();
    if ports.second == SecondScan::Complete {
        expected.extra_workspaces = vec![WorkspaceScope {
            team: TeamId("new-extra".into()), project: ProjectId("new-extra-project".into()),
        }];
    }
    assert_eq!(json!(*ports.inner.records.lock().unwrap()), json!([account(1), expected]));
    assert_eq!(json!(*ports.writes.lock().unwrap()), json!([initial, admitted, expected]));
    assert_eq!(ports.inner.state_writes.load(Ordering::SeqCst), 3);
    assert_eq!(ports.inner.session_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.inner.browser_calls.load(Ordering::SeqCst), 0);
    let saved = ports.inner.saved.lock().unwrap();
    assert_eq!(saved.len(), 1);
    ports.check_session(&saved[0]);
    let proxies = if login { vec![] } else { vec![(initial.id, initial.proxy.clone())] };
    assert_eq!(*ports.inner.registered_proxies.lock().unwrap(), proxies);
}

fn assert_calls(ports: &ScanPorts, login: bool) {
    assert_eq!(ports.scans.load(Ordering::SeqCst), 2);
    let mut expected = vec![if login { "auth_complete" } else { "probe" }, "grant"];
    if !login { expected.push("proxy"); }
    expected.extend(["upsert", "scan", "upsert", "scan"]);
    if ports.second == SecondScan::Complete { expected.push("projects"); }
    expected.push("upsert");
    let calls = ports.calls.lock().unwrap();
    let without_probes: Vec<_> = calls.iter().copied()
        .filter(|call| !call.ends_with("_usage")).collect();
    assert_eq!(without_probes, expected);
    assert_eq!(calls.iter().filter(|&&call| call == "primary_usage").count(), 2);
    assert_eq!(calls.iter().filter(|&&call| call == "extra_usage").count(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn login_incomplete_second_scan_preserves_saved_extra_scopes() {
    for second in [SecondScan::Failed, SecondScan::Partial] {
        exercise(true, second).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn import_incomplete_second_scan_preserves_saved_extra_scopes() {
    for second in [SecondScan::Failed, SecondScan::Partial] {
        exercise(false, second).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn login_complete_second_scan_refreshes_extra_scopes() {
    exercise(true, SecondScan::Complete).await;
}

#[tokio::test(flavor = "current_thread")]
async fn import_complete_second_scan_refreshes_extra_scopes() {
    exercise(false, SecondScan::Complete).await;
}
