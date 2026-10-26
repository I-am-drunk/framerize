//! Workspace observations using inert ports and manually released probe futures.
use super::*;
use crate::account::{AccountState, WorkspaceScope};
use crate::convert::{AssembledResponse, AssembledResponses, ChainEvent};
use crate::ids::{AccountId, FramerUserId};
use crate::pipeline::{InferenceRequest, InferenceResult};
use crate::port::*;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

type Verdict = Result<f64, &'static str>;
struct Probe {
    team: &'static str,
    verdict: Verdict,
    hold: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    started: AtomicBool,
    finished: AtomicBool,
}

struct Fixture {
    probes: Vec<Probe>,
    profile_error: Option<&'static str>,
    initial: ManagedAccount,
    saved: Mutex<ManagedAccount>,
    writes: Mutex<Vec<ManagedAccount>>,
    session: Session,
    updates: tokio::sync::Mutex<()>,
}

fn credit(available: f64) -> CreditSnapshot {
    CreditSnapshot {
        available,
        funds: 1,
        grants: 1,
        allocated: available,
    }
}

impl Fixture {
    fn new(spec: &[(&'static str, Verdict)]) -> Self {
        let initial = ManagedAccount {
            id: AccountId(uuid::Uuid::from_u128(0x5CA0)),
            alias: "scan-fixture".into(), email: None,
            framer_user_id: FramerUserId("scan-user".into()),
            castle_device_id: "scan-device".into(), proxy: None,
            current_team: Some(TeamId("old-team".into())),
            current_project: Some(ProjectId("old-project".into())),
            extra_workspaces: vec![WorkspaceScope {
                team: TeamId("saved-extra".into()), project: ProjectId("saved-project".into()),
            }],
            state: AccountState::Ready, credit: Some(credit(7.0)),
            consecutive_failures: 3, cooldown_step: 2, cooling_until_ms: Some(12_345),
        };
        Self {
            probes: spec.iter().map(|(team, verdict)| Probe {
                team: *team, verdict: *verdict, hold: Mutex::new(None),
                started: AtomicBool::new(false), finished: AtomicBool::new(false),
            }).collect(),
            profile_error: None,
            session: Session {
                active_user: initial.framer_user_id.clone(),
                cookie_header: "session=scan-fixture".into(),
            },
            saved: Mutex::new(initial.clone()), initial,
            writes: Mutex::new(Vec::new()), updates: tokio::sync::Mutex::new(()),
        }
    }

    fn deps(&self) -> Deps<'_> {
        Deps { client: self, browser: self, sessions: self, state: self, account_updates: &self.updates }
    }

    fn check_session(&self, session: &Session) {
        assert_eq!(session.active_user, self.session.active_user);
        assert_eq!(session.cookie_header, self.session.cookie_header);
    }

    fn hold(&self, index: usize) -> tokio::sync::oneshot::Sender<()> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        *self.probes[index].hold.lock().unwrap() = Some(receiver);
        sender
    }

    fn assert_joined(&self) {
        assert!(self.probes.iter().all(|probe| probe.started.load(Ordering::SeqCst)
            && probe.finished.load(Ordering::SeqCst)));
    }

    fn assert_unchanged(&self, current: &ManagedAccount) {
        assert_eq!(json!(current), json!(self.initial));
        assert_eq!(json!(*self.saved.lock().unwrap()), json!(self.initial));
        assert!(self.writes.lock().unwrap().is_empty());
    }
}

#[async_trait]
impl StateStore for Fixture {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        assert_eq!(id, self.initial.id);
        Ok(Some(self.saved.lock().unwrap().clone()))
    }
    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String> {
        assert_eq!(account.id, self.initial.id);
        self.writes.lock().unwrap().push(account.clone());
        *self.saved.lock().unwrap() = account.clone();
        Ok(())
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> { panic!("unexpected listing") }
    async fn remove(&self, _: AccountId) -> Result<(), String> { panic!("unexpected removal") }
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
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> { panic!("unexpected mint") }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> { panic!("unexpected login") }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> {
        panic!("unexpected login")
    }
}

#[async_trait]
impl FramerClient for Fixture {
    async fn me(&self, session: &Session) -> Result<Value, String> {
        self.check_session(session);
        if let Some(error) = self.profile_error { return Err(error.into()); }
        Ok(json!({"spaces": self.probes.iter()
            .map(|probe| json!({"ownerId": probe.team})).collect::<Vec<_>>()}))
    }

    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        self.check_session(session);
        let probe = self.probes.iter().find(|probe| probe.team == team.0).unwrap();
        assert!(!probe.started.swap(true, Ordering::SeqCst));
        let hold = probe.hold.lock().unwrap().take();
        if let Some(receiver) = hold { receiver.await.unwrap(); }
        probe.finished.store(true, Ordering::SeqCst);
        probe.verdict.map(credit).map_err(str::to_owned)
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
}

fn assert_probe_error<T: std::fmt::Debug>(result: Result<T, CoreError>, expected: &str) {
    match result {
        Err(CoreError::Framer(error)) => assert_eq!(error, expected),
        other => panic!("expected original probe error, got {other:?}"),
    }
}

async fn assert_incomplete(spec: &[(&'static str, Verdict)], claimed: &[&str], error: &str) {
    let scan = Fixture::new(spec);
    assert_probe_error(scan_workspaces(&scan.deps(), &scan.session).await, error);
    scan.assert_joined();
    scan.assert_unchanged(&scan.initial);

    let admission = Fixture::new(spec);
    let mut current = admission.initial.clone();
    let claimed = claimed.iter().map(|team| (*team).to_owned()).collect();
    assert_probe_error(
        admit(&admission.deps(), &mut current, &admission.session, &claimed).await,
        error,
    );
    admission.assert_joined();
    admission.assert_unchanged(&current);
}

#[tokio::test(flavor = "current_thread")]
async fn all_failed_preserves_first_probe_error() {
    assert_incomplete(&[("first", Err("first error")), ("last", Err("last error"))], &[], "first error").await;
}

#[tokio::test(flavor = "current_thread")]
async fn zero_and_failed_is_not_exhaustion() {
    assert_incomplete(&[("zero", Ok(0.0)), ("failed", Err("usage unavailable"))], &[], "usage unavailable").await;
}

#[tokio::test(flavor = "current_thread")]
async fn claimed_and_failed_preserves_probe_error() {
    assert_incomplete(&[("claimed", Ok(100.0)), ("failed", Err("usage unavailable"))], &["claimed"], "usage unavailable").await;
}

#[tokio::test(flavor = "current_thread")]
async fn usable_unclaimed_team_wins_despite_failed_probe() {
    let spec = [
        ("claimed", Ok(500.0)), ("low", Ok(20.0)), ("failed", Err("usage unavailable")),
        ("high", Ok(100.0)), ("tie", Ok(100.0)),
    ];
    let scan = Fixture::new(&spec);
    assert_probe_error(scan_workspaces(&scan.deps(), &scan.session).await, "usage unavailable");
    scan.assert_joined();
    scan.assert_unchanged(&scan.initial);
    let fixture = Fixture::new(&spec);
    let mut current = fixture.initial.clone();
    let claimed = BTreeSet::from(["claimed".into()]);
    assert_eq!(admit(&fixture.deps(), &mut current, &fixture.session, &claimed).await.unwrap().0, "high");
    let mut expected = fixture.initial.clone();
    expected.current_team = Some(TeamId("high".into()));
    expected.current_project = None;
    expected.credit = Some(credit(100.0));
    assert_eq!(json!(current), json!(expected));
    assert_eq!(json!(*fixture.saved.lock().unwrap()), json!(expected));
    assert_eq!(json!(*fixture.writes.lock().unwrap()), json!([expected]));
    fixture.assert_joined();
}

#[tokio::test(flavor = "current_thread")]
async fn complete_empty_and_zero_scans_keep_refusal() {
    for spec in [vec![], vec![("zero", Ok(0.0))]] {
        let scan = Fixture::new(&spec);
        assert!(scan_workspaces(&scan.deps(), &scan.session).await.unwrap().is_empty());
        scan.assert_joined();
        scan.assert_unchanged(&scan.initial);
        let fixture = Fixture::new(&spec);
        let mut current = fixture.initial.clone();
        let result = admit(&fixture.deps(), &mut current, &fixture.session, &BTreeSet::new()).await;
        match result {
            Err(CoreError::NotEligible(error)) => assert_eq!(error,
                "scan-fixture: no workspace has credits \u{2014} account cannot serve requests"),
            other => panic!("expected complete-scan refusal, got {other:?}"),
        }
        fixture.assert_joined();
        fixture.assert_unchanged(&current);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn complete_credited_scans_keep_ranking_and_claim_refusal() {
    let spec = [("low", Ok(20.0)), ("high", Ok(100.0)), ("tie", Ok(100.0))];
    let scan = Fixture::new(&spec);
    let credited = scan_workspaces(&scan.deps(), &scan.session).await.unwrap();
    assert_eq!(credited.iter().map(|w| (w.team.0.as_str(), w.snapshot.available))
        .collect::<Vec<_>>(), vec![("high", 100.0), ("tie", 100.0), ("low", 20.0)]);
    scan.assert_joined();
    scan.assert_unchanged(&scan.initial);
    let fixture = Fixture::new(&spec);
    let mut current = fixture.initial.clone();
    let claimed = spec.iter().map(|(team, _)| (*team).to_owned()).collect();
    let result = admit(&fixture.deps(), &mut current, &fixture.session, &claimed).await;
    assert!(matches!(result, Err(CoreError::NotEligible(ref error)) if error.starts_with(
        "scan-fixture: every credited workspace is already in use (high, tie, low)")));
    fixture.assert_joined();
    fixture.assert_unchanged(&current);
}

#[tokio::test(flavor = "current_thread")]
async fn public_scan_joins_delayed_probes_and_uses_input_error_order() {
    let fixture = Fixture::new(&[
        ("first", Err("first error")), ("fast", Err("fast error")), ("zero", Ok(0.0)),
    ]);
    let release = fixture.hold(0);
    let deps = fixture.deps();
    let mut scan = Box::pin(scan_workspaces(&deps, &fixture.session));
    assert!(futures::poll!(scan.as_mut()).is_pending());
    assert!(fixture.probes.iter().all(|probe| probe.started.load(Ordering::SeqCst)));
    assert!(!fixture.probes[0].finished.load(Ordering::SeqCst));
    assert!(fixture.probes[1].finished.load(Ordering::SeqCst));
    assert!(fixture.probes[2].finished.load(Ordering::SeqCst));
    release.send(()).unwrap();
    assert_probe_error(scan.await, "first error");
    fixture.assert_joined();
    fixture.assert_unchanged(&fixture.initial);
}

#[tokio::test(flavor = "current_thread")]
async fn admission_joins_delayed_probes_before_using_credited_team() {
    let fixture = Fixture::new(&[("good", Ok(100.0)), ("slow", Err("slow error"))]);
    let release = fixture.hold(1);
    let deps = fixture.deps();
    let mut current = fixture.initial.clone();
    let claimed = BTreeSet::new();
    let mut admission = Box::pin(admit(&deps, &mut current, &fixture.session, &claimed));
    assert!(futures::poll!(admission.as_mut()).is_pending());
    assert!(fixture.probes.iter().all(|probe| probe.started.load(Ordering::SeqCst)));
    assert!(fixture.writes.lock().unwrap().is_empty());
    release.send(()).unwrap();
    assert_eq!(admission.await.unwrap().0, "good");
    fixture.assert_joined();
    assert_eq!(fixture.writes.lock().unwrap().len(), 1);
    assert_eq!(current.current_team, Some(TeamId("good".into())));
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_scan_drops_all_pending_probes() {
    let fixture = Fixture::new(&[("one", Ok(100.0)), ("two", Err("pending error"))]);
    let first = fixture.hold(0);
    let second = fixture.hold(1);
    let deps = fixture.deps();
    let mut scan = Box::pin(scan_workspaces(&deps, &fixture.session));
    assert!(futures::poll!(scan.as_mut()).is_pending());
    assert!(fixture.probes.iter().all(|probe| probe.started.load(Ordering::SeqCst)));
    drop(scan);
    assert!(first.send(()).is_err());
    assert!(second.send(()).is_err());
    assert!(fixture.probes.iter().all(|probe| !probe.finished.load(Ordering::SeqCst)));
    fixture.assert_unchanged(&fixture.initial);
}

#[tokio::test(flavor = "current_thread")]
async fn profile_error_is_preserved_without_probes() {
    let mut fixture = Fixture::new(&[("unused", Ok(100.0))]);
    fixture.profile_error = Some("profile unavailable");
    let mut current = fixture.initial.clone();
    assert_probe_error(scan_workspaces(&fixture.deps(), &fixture.session).await, "profile unavailable");
    assert_probe_error(
        admit(&fixture.deps(), &mut current, &fixture.session, &BTreeSet::new()).await,
        "profile unavailable",
    );
    assert!(!fixture.probes[0].started.load(Ordering::SeqCst));
    fixture.assert_unchanged(&current);
}
