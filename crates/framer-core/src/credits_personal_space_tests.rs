//! Inert profile/probe coverage. Every future is awaited in scope and bounded.
use super::*;
use crate::account::{AccountState, WorkspaceScope};
use crate::convert::{AssembledResponse, AssembledResponses, ChainEvent};
use crate::ids::{AccountId, FramerUserId};
use crate::pipeline::{InferenceRequest, InferenceResult};
use crate::port::*;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::Duration;

const PERSONAL: &str = "12345678-90ab-4cde-8f01-012345678901";
const TEAM: &str = "12345678-90ab-4cde-8f01-012345678902";
const OTHER: &str = "12345678-90ab-4cde-8f01-012345678903";
const WAIT: Duration = Duration::from_secs(3);
type Verdict = Result<f64, &'static str>;

fn credit(available: f64) -> CreditSnapshot {
    CreditSnapshot { available, funds: 2, grants: 3, allocated: 1_500.0 }
}

impl Fixture {
    fn assert_published(&self, current: &ManagedAccount, team: &str, available: f64) {
        let mut expected = self.initial.clone();
        expected.current_team = Some(TeamId(team.into()));
        expected.current_project = None;
        expected.credit = Some(credit(available));
        assert_eq!(json!(current), json!(expected));
        assert_eq!(json!(*self.saved.lock().unwrap()), json!(expected));
        assert_eq!(json!(*self.writes.lock().unwrap()), json!([expected]));
        assert_eq!(*self.state_calls.lock().unwrap(), ["get", "list", "upsert"]);
    }

    async fn scan(&self) -> Result<Vec<WorkspaceCredit>, CoreError> {
        let deps = self.deps();
        let result = tokio::time::timeout(WAIT, scan_workspaces(&deps, &self.session)).await;
        self.assert_settled();
        result.expect("scan exceeded the fixture bound")
    }

    async fn admission(&self, current: &mut ManagedAccount, claimed: &BTreeSet<String>)
        -> Result<TeamId, CoreError> {
        let deps = self.deps();
        let result = tokio::time::timeout(WAIT, admit(&deps, current, &self.session, claimed)).await;
        self.assert_settled();
        result.expect("admission exceeded the fixture bound")
    }

    fn assert_unchanged(&self, current: &ManagedAccount) {
        assert_eq!(json!(current), json!(self.initial));
        assert_eq!(json!(*self.saved.lock().unwrap()), json!(self.initial));
        assert!(self.writes.lock().unwrap().is_empty());
        assert!(self.state_calls.lock().unwrap().is_empty());
    }

    fn deps(&self) -> Deps<'_> {
        Deps { client: self, browser: self, sessions: self, state: self, account_updates: &self.updates }
    }

    fn check_session(&self, session: &Session) {
        assert_eq!(session.active_user, self.session.active_user);
        assert_eq!(session.cookie_header, self.session.cookie_header);
    }

    fn assert_settled(&self) {
        let mut started = self.started.lock().unwrap().clone();
        let mut completed = self.completed.lock().unwrap().clone();
        started.sort();
        completed.sort();
        assert_eq!(started, completed, "every started probe must finish");
    }

    fn assert_probes(&self, expected: &[&str]) {
        assert_eq!(*self.started.lock().unwrap(),
            expected.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>());
    }

    fn new(profile: Value, verdicts: &[(&'static str, Verdict)]) -> Self {
        let initial = ManagedAccount {
            id: AccountId(uuid::Uuid::from_u128(0x5CA1)),
            alias: "personal-space-fixture".into(), email: None,
            framer_user_id: FramerUserId(PERSONAL.into()),
            castle_device_id: "personal-space-device".into(), proxy: None,
            current_team: Some(TeamId("old-team".into())),
            current_project: Some(ProjectId("old-project".into())),
            extra_workspaces: vec![WorkspaceScope {
                team: TeamId("saved-extra".into()), project: ProjectId("saved-project".into()),
            }],
            state: AccountState::CoolingDown, credit: Some(credit(7.0)),
            consecutive_failures: 3, cooldown_step: 2, cooling_until_ms: Some(12_345),
        };
        Self {
            profile, verdicts: verdicts.to_vec(),
            started: Mutex::new(Vec::new()), completed: Mutex::new(Vec::new()),
            session: Session { active_user: initial.framer_user_id.clone(),
                cookie_header: "session=personal-space-fixture".into() },
            saved: Mutex::new(initial.clone()), initial,
            writes: Mutex::new(Vec::new()), state_calls: Mutex::new(Vec::new()),
            updates: tokio::sync::Mutex::new(()),
        }
    }
}

struct Fixture {
    profile: Value,
    verdicts: Vec<(&'static str, Verdict)>,
    started: Mutex<Vec<String>>,
    completed: Mutex<Vec<String>>,
    initial: ManagedAccount,
    saved: Mutex<ManagedAccount>,
    writes: Mutex<Vec<ManagedAccount>>,
    state_calls: Mutex<Vec<&'static str>>,
    session: Session,
    updates: tokio::sync::Mutex<()>,
}

#[async_trait]
impl StateStore for Fixture {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        self.assert_settled();
        assert_eq!(id, self.initial.id);
        self.state_calls.lock().unwrap().push("get");
        Ok(Some(self.saved.lock().unwrap().clone()))
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        self.assert_settled();
        self.state_calls.lock().unwrap().push("list");
        Ok(vec![self.saved.lock().unwrap().clone()])
    }
    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String> {
        self.assert_settled();
        assert_eq!(account.id, self.initial.id);
        self.state_calls.lock().unwrap().push("upsert");
        self.writes.lock().unwrap().push(account.clone());
        *self.saved.lock().unwrap() = account.clone();
        Ok(())
    }
    async fn remove(&self, _: AccountId) -> Result<(), String> {
        panic!("unexpected removal")
    }
}

#[async_trait]
impl SessionStore for Fixture {
    async fn load(&self, _: &FramerUserId) -> Result<Option<Session>, String> {
        panic!("scan already has its session")
    }
    async fn save(&self, _: &Session) -> Result<(), String> { panic!("unexpected session save") }
    async fn delete(&self, _: &FramerUserId) -> Result<(), String> { panic!("unexpected session removal") }
}

#[async_trait]
impl BrowserBridge for Fixture {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        panic!("unexpected mint")
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("unexpected login")
    }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> {
        panic!("unexpected login")
    }
}

#[async_trait]
impl FramerClient for Fixture {
    async fn me(&self, session: &Session) -> Result<Value, String> {
        self.check_session(session);
        Ok(self.profile.clone())
    }

    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        self.check_session(session);
        self.started.lock().unwrap().push(team.0.clone());
        let verdict = self.verdicts.iter().find(|(id, _)| *id == team.0.as_str())
            .expect("unexpected credit probe").1;
        tokio::task::yield_now().await;
        self.completed.lock().unwrap().push(team.0.clone());
        verdict.map(credit).map_err(str::to_owned)
    }

    async fn run_inference(
        &self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: &InferenceRequest,
    ) -> Result<InferenceResult, String> { panic!("unexpected inference") }

    async fn proxy(
        &self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: &str, _: Value, _: &str,
    ) -> Result<ProxyResponse, String> { panic!("unexpected proxy") }

    async fn freechat_long(
        &self, _: &Session, _: &dyn BrowserBridge, _: &AccountId, _: &ProjectId,
        _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
    ) -> Result<AssembledResponse, String> { panic!("unexpected chain") }

    async fn native_long(
        &self, _: &Session, _: &dyn BrowserBridge, _: &AccountId, _: &ProjectId,
        _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
    ) -> Result<AssembledResponses, String> { panic!("unexpected chain") }

    async fn request_magic_link(&self, _: &str, _: &CastleToken) -> Result<(), String> {
        panic!("unexpected magic link")
    }
    async fn complete_magic_link(&self, _: &str, _: &CastleToken) -> Result<Session, String> {
        panic!("unexpected magic link")
    }
    async fn claim_workspace_grant(&self, _: &Session) -> Result<bool, String> {
        panic!("unexpected grant claim")
    }
    async fn team_projects(&self, _: &Session, _: &TeamId) -> Result<Vec<ProjectId>, String> {
        panic!("unexpected project lookup")
    }
    async fn create_project(&self, _: &Session, _: &TeamId) -> Result<ProjectId, String> {
        panic!("unexpected project creation")
    }
    async fn create_workspace(&self, _: &Session, _: &WorkspaceSpec) -> Result<TeamId, String> {
        panic!("unexpected workspace creation")
    }
}

fn mixed_profile(personal_first: bool) -> Value {
    let personal = json!({"scope": "user", "ownerId": PERSONAL, "id": "personal-space"});
    let organization = json!({"scope": "organization", "ownerId": TEAM, "id": "org-space"});
    let spaces = if personal_first { vec![personal, organization] } else { vec![organization, personal] };
    json!({"id": PERSONAL, "spaces": spaces})
}

fn assert_credits(actual: &[WorkspaceCredit], expected: &[(&str, f64)]) {
    let actual: Vec<_> = actual.iter().map(|w| (w.team.0.as_str(), json!(w.snapshot))).collect();
    let expected: Vec<_> = expected.iter().map(|(team, n)| (*team, json!(credit(*n)))).collect();
    assert_eq!(actual, expected);
}

fn assert_probe_error<T: std::fmt::Debug>(result: Result<T, CoreError>, expected: &str) {
    match result {
        Err(CoreError::Framer(error)) => assert_eq!(error, expected),
        other => panic!("expected original organization error, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn mixed_profile_skips_personal_space_probe() {
    for personal_first in [true, false] {
        let fixture = Fixture::new(mixed_profile(personal_first), &[
            (PERSONAL, Err("personal sentinel: 403")), (TEAM, Ok(100.0)),
        ]);
        let credited = fixture.scan().await.unwrap();
        fixture.assert_probes(&[TEAM]);
        assert_credits(&credited, &[(TEAM, 100.0)]);
        fixture.assert_unchanged(&fixture.initial);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn admission_selects_organization_without_personal_probe() {
    for personal_first in [true, false] {
        let fixture = Fixture::new(mixed_profile(personal_first), &[
            (PERSONAL, Err("personal sentinel: 403")), (TEAM, Ok(100.0)),
        ]);
        let mut current = fixture.initial.clone();
        let result = fixture.admission(&mut current, &BTreeSet::new()).await;
        assert_eq!(result.unwrap(), TeamId(TEAM.into()));
        fixture.assert_probes(&[TEAM]);
        fixture.assert_published(&current, TEAM, 100.0);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn personal_only_profile_is_complete_empty() {
    for spaces in [json!([{"scope": "user", "ownerId": PERSONAL}]), json!([
        {"scope": "user", "ownerId": PERSONAL}, {"scope": "user", "ownerId": TEAM},
        {"scope": "user"}, {"scope": "user", "ownerId": null}
    ])] {
        let profile = json!({"spaces": spaces});
        let verdicts = [(PERSONAL, Err("personal sentinel: 403")), (TEAM, Err("also personal"))];
        let scan = Fixture::new(profile.clone(), &verdicts);
        assert!(scan.scan().await.unwrap().is_empty());
        scan.assert_probes(&[]);
        scan.assert_unchanged(&scan.initial);
        let fixture = Fixture::new(profile, &verdicts);
        let mut current = fixture.initial.clone();
        let result = fixture.admission(&mut current, &BTreeSet::new()).await;
        match result {
            Err(CoreError::NotEligible(error)) => assert_eq!(error,
                "personal-space-fixture: no workspace has credits \u{2014} account cannot serve requests"),
            other => panic!("expected complete-empty refusal, got {other:?}"),
        }
        fixture.assert_probes(&[]);
        fixture.assert_unchanged(&current);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn personal_space_does_not_mask_organization_error() {
    for reversed in [false, true] {
        let (first, second, error) = if reversed {
            (OTHER, TEAM, "second organization failure")
        } else {
            (TEAM, OTHER, "first organization failure")
        };
        let profile = json!({"spaces": [
            {"scope": "user", "ownerId": PERSONAL},
            {"scope": "organization", "ownerId": first},
            {"scope": "organization", "ownerId": second}
        ]});
        let verdicts = [(PERSONAL, Err("personal sentinel: 403")),
            (TEAM, Err("first organization failure")), (OTHER, Err("second organization failure"))];
        let scan = Fixture::new(profile.clone(), &verdicts);
        assert_probe_error(scan.scan().await, error);
        scan.assert_probes(&[first, second]);
        scan.assert_unchanged(&scan.initial);
        let fixture = Fixture::new(profile, &verdicts);
        let mut current = fixture.initial.clone();
        assert_probe_error(fixture.admission(&mut current, &BTreeSet::new()).await, error);
        fixture.assert_probes(&[first, second]);
        fixture.assert_unchanged(&current);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn other_space_kinds_keep_existing_probe_contract() {
    let kinds = [
        ("unscoped", None, 10.0),
        (TEAM, Some(json!("organization")), 100.0),
        (OTHER, Some(json!("other")), 100.0),
        ("null-scope", Some(json!(null)), 20.0),
        ("bool-scope", Some(json!(false)), 20.0),
        ("number-scope", Some(json!(7)), 20.0),
        ("array-scope", Some(json!([])), 20.0),
        ("object-scope", Some(json!({})), 20.0),
        ("case-scope", Some(json!("User")), 20.0),
        ("space-scope", Some(json!(" user ")), 20.0),
        ("empty-scope", Some(json!("")), 20.0),
        ("12345678", None, 20.0),
        (TEAM, Some(json!("organization")), 100.0),
        ("", None, 5.0),
    ];
    let mut spaces: Vec<Value> = kinds.iter().map(|(owner, scope, _)| {
        let mut space = json!({"ownerId": owner});
        if let Some(scope) = scope { space["scope"] = scope.clone(); }
        space
    }).collect();
    spaces.extend([json!({"scope": "organization"}), json!({"ownerId": null}),
        json!({"ownerId": false}), json!({"ownerId": 7}), json!({"ownerId": []}),
        json!({"ownerId": {}}), json!([{"ownerId": "hidden"}]), json!(null)]);
    let verdicts: Vec<_> = kinds.iter().map(|(id, _, n)| (*id, Ok(*n))).collect();
    let fixture = Fixture::new(json!({"spaces": spaces}), &verdicts);
    let credited = fixture.scan().await.unwrap();
    fixture.assert_probes(&kinds.iter().map(|(id, _, _)| *id).collect::<Vec<_>>());
    assert_credits(&credited, &[(TEAM, 100.0), (OTHER, 100.0), (TEAM, 100.0),
        ("null-scope", 20.0), ("bool-scope", 20.0), ("number-scope", 20.0),
        ("array-scope", 20.0), ("object-scope", 20.0), ("case-scope", 20.0),
        ("space-scope", 20.0), ("empty-scope", 20.0), ("12345678", 20.0),
        ("unscoped", 10.0), ("", 5.0)]);
    fixture.assert_unchanged(&fixture.initial);
}
