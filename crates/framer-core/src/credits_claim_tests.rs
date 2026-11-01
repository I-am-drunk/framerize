//! Inert admission publication checks. No browser, network or filesystem I/O.
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
use tokio::sync::oneshot;

#[derive(Clone, Copy, PartialEq, Debug)]
enum StoreCall { Get, List, Upsert }
type Verdict = Result<f64, &'static str>;
struct Fixture {
    initial: ManagedAccount,
    records: Mutex<Vec<ManagedAccount>>,
    writes: Mutex<Vec<ManagedAccount>>,
    calls: Mutex<Vec<StoreCall>>,
    fail: Mutex<Option<StoreCall>>,
    pause: Mutex<Option<(StoreCall, oneshot::Receiver<()>)>>,
    probe: Mutex<Option<oneshot::Receiver<()>>>,
    workspaces: Vec<(&'static str, Verdict)>,
    session: Session,
    updates: tokio::sync::Mutex<()>,
}

fn credit(available: f64) -> CreditSnapshot {
    CreditSnapshot { available, funds: 1, grants: 1, allocated: available }
}

impl Fixture {
    fn new(workspaces: &[(&'static str, Verdict)]) -> Self {
        let initial = ManagedAccount {
            id: AccountId(uuid::Uuid::from_u128(0xC1A1)),
            alias: "claim-fixture".into(), email: None,
            framer_user_id: FramerUserId("claim-user".into()),
            castle_device_id: "claim-device".into(), proxy: None,
            current_team: Some(TeamId("old-team".into())),
            current_project: Some(ProjectId("old-project".into())),
            extra_workspaces: vec![WorkspaceScope {
                team: TeamId("saved-extra".into()), project: ProjectId("saved-project".into()),
            }],
            state: AccountState::Ready, credit: Some(credit(7.0)),
            consecutive_failures: 3, cooldown_step: 2, cooling_until_ms: Some(12_345), cooldown_cause: None,
        };
        Self {
            session: Session { active_user: initial.framer_user_id.clone(),
                cookie_header: "session=claim-fixture".into() },
            records: Mutex::new(vec![initial.clone()]), initial,
            writes: Mutex::new(Vec::new()), calls: Mutex::new(Vec::new()),
            fail: Mutex::new(None), pause: Mutex::new(None), probe: Mutex::new(None),
            workspaces: workspaces.to_vec(), updates: tokio::sync::Mutex::new(()),
        }
    }

    fn deps(&self) -> Deps<'_> {
        Deps { client: self, browser: self, sessions: self, state: self,
            account_updates: &self.updates }
    }

    fn peer(&self, team: &str, extra: bool) {
        let mut peer = self.initial.clone();
        peer.id = AccountId(uuid::Uuid::from_u128(0xC1A2));
        peer.framer_user_id = FramerUserId("peer-user".into());
        peer.castle_device_id = "peer-device".into();
        peer.current_team = (!extra).then(|| TeamId(team.into()));
        peer.current_project = None;
        peer.extra_workspaces = if extra { vec![WorkspaceScope {
            team: TeamId(team.into()), project: ProjectId("peer-project".into()),
        }] } else { Vec::new() };
        self.records.lock().unwrap().push(peer);
    }

    async fn late(&self, current: &mut ManagedAccount, claimed: &BTreeSet<String>,
        change: impl FnOnce(&Self)) -> Result<TeamId, CoreError> {
        let (release, hold) = oneshot::channel();
        *self.probe.lock().unwrap() = Some(hold);
        let deps = self.deps();
        let mut pending = Box::pin(admit(&deps, current, &self.session, claimed));
        assert!(futures::poll!(pending.as_mut()).is_pending());
        assert!(self.calls.lock().unwrap().is_empty());
        { let _writer = self.updates.try_lock().expect("probe held publication lock");
          change(self); }
        release.send(()).unwrap();
        bounded(pending).await
    }

    async fn at(&self, call: StoreCall) -> Result<(), String> {
        self.calls.lock().unwrap().push(call);
        assert!(self.updates.try_lock().is_err(), "unlocked publication {call:?}");
        let hold = {
            let mut pause = self.pause.lock().unwrap();
            if pause.as_ref().is_some_and(|(stage, _)| *stage == call) {
                pause.take().map(|(_, hold)| hold)
            } else { None }
        };
        if let Some(hold) = hold { hold.await.expect("fixture hold abandoned"); }
        if *self.fail.lock().unwrap() == Some(call) {
            return Err(format!("fixture {call:?} failure"));
        }
        Ok(())
    }

    fn assert_subject(&self, expected: &ManagedAccount) {
        let records = self.records.lock().unwrap();
        let saved = records.iter().find(|record| record.id == expected.id).unwrap();
        assert_eq!(json!(saved), json!(expected));
    }
}

#[async_trait]
impl StateStore for Fixture {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        self.at(StoreCall::Get).await?;
        Ok(self.records.lock().unwrap().iter().find(|record| record.id == id).cloned())
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        self.at(StoreCall::List).await?;
        Ok(self.records.lock().unwrap().clone())
    }
    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String> {
        self.writes.lock().unwrap().push(account.clone());
        self.at(StoreCall::Upsert).await?;
        let mut records = self.records.lock().unwrap();
        *records.iter_mut().find(|record| record.id == account.id).unwrap() = account.clone();
        Ok(())
    }
    async fn remove(&self, _: AccountId) -> Result<(), String> { panic!("unexpected removal") }
}

#[async_trait]
impl SessionStore for Fixture {
    async fn load(&self, _: &FramerUserId) -> Result<Option<Session>, String> { panic!("unexpected load") }
    async fn save(&self, _: &Session) -> Result<(), String> { panic!("unexpected save") }
    async fn delete(&self, _: &FramerUserId) -> Result<(), String> { panic!("unexpected delete") }
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
        assert_eq!(session.active_user, self.session.active_user);
        assert_eq!(session.cookie_header, self.session.cookie_header);
        Ok(json!({"spaces": self.workspaces.iter()
            .map(|(team, _)| json!({"ownerId": team})).collect::<Vec<_>>()}))
    }
    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        assert_eq!(session.active_user, self.session.active_user);
        let hold = self.probe.lock().unwrap().take();
        if let Some(hold) = hold { hold.await.expect("fixture probe abandoned"); }
        self.workspaces.iter().find(|(id, _)| *id == team.0).unwrap().1
            .map(credit).map_err(str::to_owned)
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

#[tokio::test]
async fn late_primary_claim_selects_the_richest_remaining_team() {
    let fixture = Fixture::new(&[("shared", Ok(100.0)), ("low", Ok(10.0)), ("backup", Ok(50.0))]);
    let mut current = fixture.initial.clone();
    let result = fixture.late(&mut current, &BTreeSet::new(), |f| f.peer("shared", false)).await;
    assert_eq!(result.unwrap().0, "backup");
    let mut expected = fixture.initial.clone();
    expected.current_team = Some(TeamId("backup".into()));
    expected.current_project = None;
    expected.credit = Some(credit(50.0));
    assert_eq!(json!(current), json!(expected));
    fixture.assert_subject(&expected);
}

#[tokio::test]
async fn late_extra_claim_preserves_latest_health_and_same_team_project() {
    let fixture = Fixture::new(&[("shared", Ok(100.0)), ("old-team", Ok(40.0))]);
    let mut current = fixture.initial.clone();
    let result = fixture.late(&mut current, &BTreeSet::new(), |f| {
        f.peer("shared", true);
        let mut records = f.records.lock().unwrap();
        records[0].consecutive_failures = 9;
        records[0].cooldown_step = 5;
        records[0].cooling_until_ms = Some(99_000);
        records[0].state = AccountState::CoolingDown;
        records[0].alias = "new alias".into();
    }).await;
    assert_eq!(result.unwrap().0, "old-team");
    let mut expected = fixture.initial.clone();
    expected.consecutive_failures = 9;
    expected.cooldown_step = 5;
    expected.cooling_until_ms = Some(99_000);
    expected.state = AccountState::CoolingDown;
    expected.alias = "new alias".into();
    expected.credit = Some(credit(40.0));
    assert_eq!(json!(current), json!(expected));
    fixture.assert_subject(&expected);
}

#[tokio::test]
async fn supplied_claims_remain_excluded_after_the_peer_recheck() {
    let fixture = Fixture::new(&[("supplied", Ok(200.0)), ("shared", Ok(100.0)), ("backup", Ok(50.0))]);
    let mut current = fixture.initial.clone();
    let result = fixture.late(&mut current, &BTreeSet::from(["supplied".into()]),
        |f| f.peer("shared", false)).await;
    assert_eq!(result.unwrap().0, "backup");
}

#[tokio::test]
async fn own_claims_are_excluded_and_peer_team_ids_are_compared_in_full() {
    let fixture = Fixture::new(&[("same-prefix-a", Ok(200.0)), ("same-prefix-b", Ok(100.0))]);
    let mut current = fixture.initial.clone();
    let result = fixture.late(&mut current, &BTreeSet::new(), |f| {
        f.peer("same-prefix-a", false);
        f.records.lock().unwrap()[0].extra_workspaces.push(WorkspaceScope {
            team: TeamId("same-prefix-b".into()), project: ProjectId("own-project".into()),
        });
    }).await;
    assert_eq!(result.unwrap().0, "same-prefix-b");
    assert_eq!(current.extra_workspaces.len(), 2);
    fixture.assert_subject(&current);
}

#[tokio::test]
async fn late_claim_preserves_the_first_probe_error_without_publishing() {
    let fixture = Fixture::new(&[("shared", Ok(100.0)),
        ("failed-first", Err("first probe failed")), ("failed-second", Err("second probe failed"))]);
    let mut current = fixture.initial.clone();
    let result = fixture.late(&mut current, &BTreeSet::new(), |f| f.peer("shared", true)).await;
    assert!(matches!(result, Err(CoreError::Framer(error)) if error == "first probe failed"));
    assert_eq!(json!(current), json!(fixture.initial));
    fixture.assert_subject(&fixture.initial);
    assert!(fixture.writes.lock().unwrap().is_empty());
    assert_eq!(*fixture.calls.lock().unwrap(), [StoreCall::Get, StoreCall::List]);
}

#[tokio::test]
async fn complete_late_claim_refuses_without_changing_the_subject() {
    let fixture = Fixture::new(&[("shared", Ok(100.0))]);
    let mut current = fixture.initial.clone();
    let result = fixture.late(&mut current, &BTreeSet::new(), |f| f.peer("shared", false)).await;
    assert!(matches!(result, Err(CoreError::NotEligible(error))
        if error.contains("every credited workspace is already in use (shared)")));
    assert_eq!(json!(current), json!(fixture.initial));
    fixture.assert_subject(&fixture.initial);
    assert!(fixture.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn already_refused_scans_do_not_read_or_write_state() {
    for workspaces in [vec![], vec![("zero", Ok(0.0))], vec![("claimed", Ok(100.0))]] {
        let fixture = Fixture::new(&workspaces);
        *fixture.fail.lock().unwrap() = Some(StoreCall::Get);
        let mut current = fixture.initial.clone();
        let result = admit(&fixture.deps(), &mut current, &fixture.session,
            &BTreeSet::from(["claimed".into()])).await;
        assert!(matches!(result, Err(CoreError::NotEligible(_))));
        assert!(fixture.calls.lock().unwrap().is_empty());
        assert!(fixture.writes.lock().unwrap().is_empty());
        assert_eq!(json!(current), json!(fixture.initial));
    }
}

async fn store_failure(stage: StoreCall) {
    let fixture = Fixture::new(&[("shared", Ok(100.0))]);
    let mut current = fixture.initial.clone();
    let result = fixture.late(&mut current, &BTreeSet::new(), |f| {
        *f.fail.lock().unwrap() = Some(stage);
    }).await;
    assert!(matches!(result, Err(CoreError::Store(error))
        if error == format!("fixture {stage:?} failure")));
    assert_eq!(json!(current), json!(fixture.initial));
    fixture.assert_subject(&fixture.initial);
    let expected = match stage {
        StoreCall::Get => vec![StoreCall::Get],
        StoreCall::List => vec![StoreCall::Get, StoreCall::List],
        StoreCall::Upsert => vec![StoreCall::Get, StoreCall::List, StoreCall::Upsert],
    };
    let calls = fixture.calls.lock().unwrap();
    if stage == StoreCall::Upsert {
        assert_eq!(calls.first(), Some(&StoreCall::Get));
        assert_eq!(calls.last(), Some(&StoreCall::Upsert));
    } else {
        assert_eq!(*calls, expected);
    }
    assert_eq!(fixture.writes.lock().unwrap().len(), usize::from(stage == StoreCall::Upsert));
    assert!(fixture.updates.try_lock().is_ok());
}

#[tokio::test]
async fn late_get_failure_preserves_caller_and_store() { store_failure(StoreCall::Get).await; }

#[tokio::test]
async fn late_list_failure_preserves_caller_and_store() { store_failure(StoreCall::List).await; }

#[tokio::test]
async fn late_upsert_failure_preserves_caller_and_store() { store_failure(StoreCall::Upsert).await; }

#[tokio::test]
async fn removed_subject_keeps_its_existing_error_before_peer_read() {
    let fixture = Fixture::new(&[("shared", Ok(100.0))]);
    let mut current = fixture.initial.clone();
    let result = fixture.late(&mut current, &BTreeSet::new(), |f| {
        f.records.lock().unwrap().clear();
    }).await;
    assert!(matches!(result, Err(CoreError::NotEligible(error))
        if error == "account removed during update"));
    assert_eq!(json!(current), json!(fixture.initial));
    assert_eq!(*fixture.calls.lock().unwrap(), [StoreCall::Get]);
    assert!(fixture.records.lock().unwrap().is_empty());
    assert!(fixture.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn every_subject_scope_check_precedes_the_peer_read() {
    let changes: [fn(&mut ManagedAccount); 5] = [
        |a| a.framer_user_id = FramerUserId("replacement-user".into()),
        |a| a.castle_device_id = "replacement-device".into(),
        |a| a.current_team = Some(TeamId("replacement-team".into())),
        |a| a.current_project = Some(ProjectId("replacement-project".into())),
        |a| a.proxy = Some("http://fixture.invalid:9000".into()),
    ];
    for change in changes {
        let fixture = Fixture::new(&[("shared", Ok(100.0))]);
        let mut current = fixture.initial.clone();
        let result = fixture.late(&mut current, &BTreeSet::new(), |f| {
            change(&mut f.records.lock().unwrap()[0]);
            *f.fail.lock().unwrap() = Some(StoreCall::List);
        }).await;
        assert!(matches!(result, Err(CoreError::NotEligible(error))
            if error == "account scope changed during update"));
        assert_eq!(json!(current), json!(fixture.initial));
        let mut expected = fixture.initial.clone();
        change(&mut expected);
        fixture.assert_subject(&expected);
        assert_eq!(*fixture.calls.lock().unwrap(), [StoreCall::Get]);
        assert!(fixture.writes.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn publication_lock_covers_peer_read_through_completed_save() {
    for stage in [StoreCall::List, StoreCall::Upsert] {
        let fixture = Fixture::new(&[("shared", Ok(100.0))]);
        let (release, hold) = oneshot::channel();
        *fixture.pause.lock().unwrap() = Some((stage, hold));
        let mut current = fixture.initial.clone();
        let deps = fixture.deps();
        let claimed = BTreeSet::new();
        let mut pending = Box::pin(admit(&deps, &mut current, &fixture.session, &claimed));
        assert!(futures::poll!(pending.as_mut()).is_pending());
        let mut waiting_writer = Box::pin(fixture.updates.lock());
        assert!(futures::poll!(waiting_writer.as_mut()).is_pending());
        fixture.assert_subject(&fixture.initial);
        release.send(()).unwrap();
        assert_eq!(bounded(pending).await.unwrap().0, "shared");
        let _writer = bounded(waiting_writer).await;
        fixture.assert_subject(&current);
    }
}

#[tokio::test]
async fn cancelled_peer_read_releases_publication_lock_without_writing() {
    let fixture = Fixture::new(&[("shared", Ok(100.0))]);
    let (release, hold) = oneshot::channel();
    *fixture.pause.lock().unwrap() = Some((StoreCall::List, hold));
    let mut current = fixture.initial.clone();
    let deps = fixture.deps();
    let claimed = BTreeSet::new();
    let mut pending = Box::pin(admit(&deps, &mut current, &fixture.session, &claimed));
    assert!(futures::poll!(pending.as_mut()).is_pending());
    assert!(fixture.updates.try_lock().is_err());
    drop(pending);
    assert!(release.send(()).is_err());
    assert!(fixture.updates.try_lock().is_ok());
    assert_eq!(json!(current), json!(fixture.initial));
    fixture.assert_subject(&fixture.initial);
    assert!(fixture.writes.lock().unwrap().is_empty());
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(3), future)
        .await.expect("inert admission did not settle")
}
