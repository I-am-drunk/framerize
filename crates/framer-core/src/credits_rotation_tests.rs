//! Public rotation contracts using inert ports, with no tasks, sleeps or I/O.
use crate::account::{AccountState, ManagedAccount, WorkspaceScope};
use crate::convert::{AssembledResponse, AssembledResponses, ChainEvent};
use crate::credits::{ensure_workspace, rotate_workspace};
use crate::error::CoreError;
use crate::ids::{AccountId, FramerUserId, ProjectId, TeamId};
use crate::pipeline::{Deps, InferenceRequest, InferenceResult};
use crate::port::*;
use crate::workspace::CreditSnapshot;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Mutex;

const TEAM_A: &str = "00000000-0000-4000-8000-000000000001";
const TEAM_B: &str = "00000000-0000-4000-8000-000000000002";
const TEAM_C: &str = "00000000-0000-4000-8000-000000000003";
const PROJECT_A: &str = "RotationProjectA0001";
const PROJECT_B: &str = "RotationProjectB0001";
const PROJECT_C: &str = "RotationProjectC0001";

#[derive(Clone, Copy, Debug)]
enum Entry { Ensure, Rotate }
const ENTRIES: [Entry; 2] = [Entry::Ensure, Entry::Rotate];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Boundary { CurrentUsage, Profile, SelectedUsage, Projects }

fn credit(available: f64) -> CreditSnapshot {
    CreditSnapshot { available, funds: 1, grants: 1, allocated: 100.0 }
}

fn scope(team: &str, project: &str) -> WorkspaceScope {
    WorkspaceScope { team: TeamId(team.into()), project: ProjectId(project.into()) }
}

fn assert_record(actual: &ManagedAccount, expected: &ManagedAccount) {
    assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(expected).unwrap());
}

fn account(index: u8) -> ManagedAccount {
    ManagedAccount {
        id: AccountId(uuid::Uuid::from_u128(0xCC00 + u128::from(index))),
        alias: format!("rotation-fixture-{index}"), email: None,
        framer_user_id: FramerUserId(format!("rotation-user-{index}")),
        castle_device_id: format!("rotation-device-{index}"), proxy: None,
        current_team: Some(TeamId(TEAM_A.into())),
        current_project: Some(ProjectId(PROJECT_A.into())),
        extra_workspaces: vec![], state: AccountState::Ready,
        credit: Some(credit(0.0)), consecutive_failures: 0,
        cooldown_step: 0, cooling_until_ms: None, cooldown_cause: None,
    }
}

fn assert_pair(record: &ManagedAccount, team: &str, project: &str) {
    assert_eq!(record.current_team.as_ref().map(|t| t.0.as_str()), Some(team));
    assert_eq!(record.current_project.as_ref().map(|p| p.0.as_str()), Some(project));
}

struct Fixture {
    initial: ManagedAccount,
    other: ManagedAccount,
    session: Session,
    records: Mutex<Vec<ManagedAccount>>,
    writes: Mutex<Vec<ManagedAccount>>,
    projects: Option<Result<Vec<ProjectId>, String>>,
    project_calls: Mutex<usize>,
    change: Mutex<Option<(Boundary, Option<ManagedAccount>)>>,
    updates: tokio::sync::Mutex<()>,
}

impl Fixture {
    fn new(current: &ManagedAccount) -> Self {
        let mut other = account(if current.id == account(1).id { 2 } else { 1 });
        other.current_team = Some(TeamId(TEAM_B.into()));
        other.current_project = Some(ProjectId(PROJECT_C.into()));
        other.extra_workspaces = vec![scope(TEAM_B, PROJECT_C)];
        Self {
            initial: current.clone(), other: other.clone(),
            session: Session { active_user: current.framer_user_id.clone(),
                cookie_header: format!("session=inert-{}", current.framer_user_id.0) },
            records: Mutex::new(vec![current.clone(), other]),
            writes: Mutex::new(vec![]),
            projects: Some(Ok(vec![ProjectId(PROJECT_B.into())])),
            project_calls: Mutex::new(0), change: Mutex::new(None),
            updates: tokio::sync::Mutex::new(()),
        }
    }

    fn stored(&self, id: AccountId) -> Option<ManagedAccount> {
        self.records.lock().unwrap().iter().find(|a| a.id == id).cloned()
    }

    fn check_session(&self, session: &Session) {
        assert_eq!(session.active_user, self.initial.framer_user_id);
        assert_eq!(session.cookie_header, self.session.cookie_header);
    }

    fn change_at(&self, at: Boundary, record: Option<ManagedAccount>) {
        *self.change.lock().unwrap() = Some((at, record));
    }

    async fn boundary(&self, at: Boundary) {
        let change = {
            let mut pending = self.change.lock().unwrap();
            if pending.as_ref().is_some_and(|(when, _)| *when == at) {
                pending.take()
            } else { None }
        };
        if let Some((_, replacement)) = change {
            // A concurrent writer completes under the same lock before this reply.
            let _update = self.updates.lock().await;
            let mut records = self.records.lock().unwrap();
            records.retain(|a| a.id != self.initial.id);
            if let Some(record) = replacement {
                assert_eq!(record.id, self.initial.id);
                records.push(record);
            }
        }
    }

    fn finish(&self, project_calls: usize) {
        assert_eq!(*self.project_calls.lock().unwrap(), project_calls);
        self.assert_isolated();
    }

    fn assert_isolated(&self) {
        assert!(*self.project_calls.lock().unwrap() <= 1);
        assert!(self.change.lock().unwrap().is_none(), "reply boundary was not reached");
        assert_record(&self.stored(self.other.id).unwrap(), &self.other);
    }

    fn assert_rotated(&self, current: &ManagedAccount) {
        assert_pair(current, TEAM_B, PROJECT_B);
        assert_record(&self.stored(current.id).unwrap(), current);
        assert_eq!(current.credit.as_ref().unwrap().available, 80.0);
        let writes = self.writes.lock().unwrap();
        let mut publications = 0;
        for record in writes.iter() {
            if record.current_team == Some(TeamId(TEAM_B.into())) {
                assert_pair(record, TEAM_B, PROJECT_B);
                publications += 1;
            } else {
                assert_pair(record, TEAM_A, PROJECT_A);
            }
        }
        assert_eq!(publications, 1, "publish the new pair together exactly once");
    }

    fn assert_unchanged(&self, current: &ManagedAccount) {
        assert_record(current, &self.initial);
        assert_record(&self.stored(current.id).unwrap(), &self.initial);
        for record in self.writes.lock().unwrap().iter() {
            assert_pair(record, TEAM_A, PROJECT_A);
        }
    }

    fn assert_guarded(&self, current: &ManagedAccount, replacement: Option<&ManagedAccount>) {
        assert_record(current, &self.initial);
        match replacement {
            Some(expected) => assert_record(&self.stored(current.id).unwrap(), expected),
            None => assert!(self.stored(current.id).is_none()),
        }
        for written in self.writes.lock().unwrap().iter() {
            assert_pair(written, TEAM_A, PROJECT_A);
        }
        self.assert_isolated();
    }

    async fn run(&self, entry: Entry, current: &mut ManagedAccount) -> Result<TeamId, CoreError> {
        let deps = Deps { client: self, browser: self, sessions: self,
            state: self, account_updates: &self.updates };
        match entry {
            Entry::Ensure => ensure_workspace(&deps, current, &self.session).await,
            Entry::Rotate => rotate_workspace(&deps, current, &self.session).await,
        }
    }
}

#[async_trait]
impl StateStore for Fixture {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        Ok(self.stored(id))
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        Ok(self.records.lock().unwrap().clone())
    }
    async fn upsert(&self, record: &ManagedAccount) -> Result<(), String> {
        assert_eq!(record.id, self.initial.id, "rotation must not write another account");
        self.writes.lock().unwrap().push(record.clone());
        let mut records = self.records.lock().unwrap();
        records.retain(|a| a.id != record.id);
        records.push(record.clone());
        Ok(())
    }
    async fn remove(&self, _: AccountId) -> Result<(), String> {
        panic!("unexpected account removal")
    }
}

#[async_trait]
impl SessionStore for Fixture {
    async fn load(&self, _: &FramerUserId) -> Result<Option<Session>, String> {
        panic!("rotation already has its account session")
    }
    async fn save(&self, _: &Session) -> Result<(), String> {
        panic!("unexpected session write")
    }
    async fn delete(&self, _: &FramerUserId) -> Result<(), String> {
        panic!("unexpected session removal")
    }
}

#[async_trait]
impl BrowserBridge for Fixture {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        panic!("unexpected token mint")
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
        self.boundary(Boundary::Profile).await;
        Ok(json!({"spaces": [{"ownerId": TEAM_A}, {"ownerId": TEAM_B}]}))
    }

    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        self.check_session(session);
        let (boundary, available) = if team.0 == TEAM_A {
            (Boundary::CurrentUsage, 0.0)
        } else {
            assert_eq!(team.0, TEAM_B);
            (Boundary::SelectedUsage, 80.0)
        };
        self.boundary(boundary).await;
        Ok(credit(available))
    }

    async fn team_projects(&self, session: &Session, team: &TeamId) -> Result<Vec<ProjectId>, String> {
        self.check_session(session);
        assert_eq!(team.0, TEAM_B);
        for written in self.writes.lock().unwrap().iter() {
            // Resolution must precede any publication of the selected workspace.
            assert_pair(written, TEAM_A, PROJECT_A);
        }
        *self.project_calls.lock().unwrap() += 1;
        self.boundary(Boundary::Projects).await;
        self.projects.clone().expect("an applicable saved scope must avoid listing")
    }

    async fn request_magic_link(&self, _: &str, _: &CastleToken) -> Result<(), String> {
        panic!("unexpected magic link")
    }
    async fn complete_magic_link(&self, _: &str, _: &CastleToken) -> Result<Session, String> {
        panic!("unexpected magic link")
    }
    async fn create_project(&self, _: &Session, _: &TeamId) -> Result<ProjectId, String> {
        panic!("rotation must not create a project")
    }
    async fn create_workspace(&self, _: &Session, _: &WorkspaceSpec) -> Result<TeamId, String> {
        panic!("these existing-team rotations must not create a workspace")
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
        panic!("unexpected free chat")
    }
    async fn native_long(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>) -> Result<AssembledResponses, String> {
        panic!("unexpected native chat")
    }
}

fn rotated(mut record: ManagedAccount) -> ManagedAccount {
    record.current_team = Some(TeamId(TEAM_B.into()));
    record.current_project = Some(ProjectId(PROJECT_B.into()));
    record.credit = Some(credit(80.0));
    record.extra_workspaces.retain(|scope| scope.team.0 != TEAM_B);
    record
}

#[tokio::test]
async fn rotation_prefers_applicable_saved_scope_over_listing() {
    for entry in ENTRIES {
        let mut current = account(1);
        current.extra_workspaces = vec![scope(TEAM_C, PROJECT_C), scope(TEAM_B, PROJECT_B)];
        let mut fixture = Fixture::new(&current);
        fixture.projects = None;
        assert_eq!(fixture.run(entry, &mut current).await.unwrap(), TeamId(TEAM_B.into()));
        fixture.assert_rotated(&current);
        assert_record(&current, &rotated(fixture.initial.clone()));
        let scopes: Vec<_> = current.all_scopes().into_iter()
            .map(|scope| (scope.team.0, scope.project.0)).collect();
        assert_eq!(scopes, vec![(TEAM_B.to_owned(), PROJECT_B.to_owned()),
            (TEAM_C.to_owned(), PROJECT_C.to_owned())]);
        fixture.finish(0);
    }
}

#[tokio::test]
async fn rotation_lists_for_the_same_account_and_ignores_other_account_scopes() {
    for entry in ENTRIES {
        for owner in [1, 2] {
            let mut current = account(owner);
            current.extra_workspaces = vec![scope(TEAM_C, PROJECT_C)];
            let fixture = Fixture::new(&current);
            assert_eq!(fixture.run(entry, &mut current).await.unwrap(), TeamId(TEAM_B.into()));
            fixture.assert_rotated(&current);
            assert_record(&current, &rotated(fixture.initial.clone()));
            fixture.finish(1);
        }
    }
}

#[tokio::test]
async fn absent_project_refuses_without_changing_the_old_pair() {
    for entry in ENTRIES {
        let mut current = account(1);
        let mut fixture = Fixture::new(&current);
        fixture.projects = Some(Ok(vec![]));
        assert!(fixture.run(entry, &mut current).await.is_err());
        fixture.assert_unchanged(&current);
        fixture.finish(1);
    }
}

#[tokio::test]
async fn failed_project_listing_refuses_without_changing_the_old_pair() {
    for entry in ENTRIES {
        let mut current = account(1);
        let mut fixture = Fixture::new(&current);
        fixture.projects = Some(Err("inert project listing failure".into()));
        assert!(fixture.run(entry, &mut current).await.is_err());
        fixture.assert_unchanged(&current);
        fixture.finish(1);
    }
}

#[tokio::test]
async fn publication_preserves_newer_health_cooldown_and_unrelated_fields() {
    for entry in ENTRIES {
        for (saved, boundary) in [(true, Boundary::SelectedUsage),
            (false, Boundary::SelectedUsage), (false, Boundary::Projects)] {
            let mut current = account(1);
            if saved { current.extra_workspaces.push(scope(TEAM_B, PROJECT_B)); }
            let mut newer = current.clone();
            newer.cooling_until_ms = Some(1_000_000);
            newer.cooldown_step = 4;
            newer.consecutive_failures = 9;
            newer.alias = "newer label".into();
            newer.email = Some("updated@example.test".into());
            let mut fixture = Fixture::new(&current);
            if saved { fixture.projects = None; }
            fixture.change_at(boundary, Some(newer.clone()));
            assert_eq!(fixture.run(entry, &mut current).await.unwrap(), TeamId(TEAM_B.into()));
            fixture.assert_rotated(&current);
            assert_record(&current, &rotated(newer));
            assert!(!current.is_available_at(999_999));
            assert!(current.is_available_at(1_000_000));
            fixture.finish(if saved { 0 } else { 1 });
        }
    }
}

#[tokio::test]
async fn rotation_never_revives_a_concurrently_disabled_account() {
    for state in [AccountState::Disconnected, AccountState::Authenticating,
        AccountState::CoolingDown, AccountState::Exhausted, AccountState::Quarantined] {
        for entry in ENTRIES {
            let mut current = account(1);
            let mut newer = current.clone();
            newer.state = state.clone();
            newer.cooling_until_ms = Some(1_000_000);
            newer.cooldown_step = 3;
            newer.consecutive_failures = 7;
            let fixture = Fixture::new(&current);
            fixture.change_at(Boundary::Projects, Some(newer.clone()));
            let result = fixture.run(entry, &mut current).await;
            // Refusal is also safe; publishing a pair never grants health authority.
            let expected = if result.is_ok() { rotated(newer) } else { newer };
            let stored = fixture.stored(current.id).unwrap();
            assert_record(&stored, &expected);
            assert!(!stored.is_eligible());
            assert!(!stored.is_available_at(1_000_000));
            if result.is_ok() { fixture.assert_rotated(&current); }
            else { assert_record(&current, &fixture.initial); }
            fixture.finish(1);
        }
    }
}

fn change_scope(record: &mut ManagedAccount, dimension: &str) {
    match dimension {
        "user" => record.framer_user_id = FramerUserId("replacement-user".into()),
        "device" => record.castle_device_id = "replacement-device".into(),
        "team" => record.current_team = Some(TeamId(TEAM_C.into())),
        "project" => record.current_project = Some(ProjectId(PROJECT_C.into())),
        "proxy" => record.proxy = Some("http://replacement.invalid:8080".into()),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn changed_account_scope_refuses_stale_rotation_publication() {
    for entry in ENTRIES {
        for dimension in ["user", "device", "team", "project", "proxy"] {
            for (saved, boundary) in [(true, Boundary::SelectedUsage),
                (false, Boundary::Profile), (false, Boundary::SelectedUsage),
                (false, Boundary::Projects)] {
                let mut current = account(1);
                if saved { current.extra_workspaces.push(scope(TEAM_B, PROJECT_B)); }
                let mut newer = current.clone();
                change_scope(&mut newer, dimension);
                let mut fixture = Fixture::new(&current);
                if saved { fixture.projects = None; }
                fixture.change_at(boundary, Some(newer.clone()));
                assert!(matches!(fixture.run(entry, &mut current).await,
                    Err(CoreError::NotEligible(_))), "changed {dimension}, {entry:?}");
                fixture.assert_guarded(&current, Some(&newer));
            }
        }
    }
}

#[tokio::test]
async fn changed_saved_scope_cannot_publish_its_stale_project() {
    for entry in ENTRIES {
        for dimension in ["removed", "team", "project"] {
            let mut current = account(1);
            current.extra_workspaces.push(scope(TEAM_B, PROJECT_B));
            let mut newer = current.clone();
            match dimension {
                "removed" => newer.extra_workspaces.clear(),
                "team" => newer.extra_workspaces[0].team = TeamId(TEAM_C.into()),
                "project" => newer.extra_workspaces[0].project = ProjectId(PROJECT_C.into()),
                _ => unreachable!(),
            }
            let mut fixture = Fixture::new(&current);
            fixture.projects = Some(Err("no replacement project evidence".into()));
            fixture.change_at(Boundary::SelectedUsage, Some(newer.clone()));
            assert!(fixture.run(entry, &mut current).await.is_err(), "changed {dimension}");
            fixture.assert_guarded(&current, Some(&newer));
        }
    }
}

#[tokio::test]
async fn removed_accounts_are_never_recreated_by_rotation() {
    for entry in ENTRIES {
        for boundary in [Boundary::Profile, Boundary::SelectedUsage, Boundary::Projects] {
            let mut current = account(1);
            let fixture = Fixture::new(&current);
            fixture.change_at(boundary, None);
            assert!(matches!(fixture.run(entry, &mut current).await,
                Err(CoreError::NotEligible(_))));
            fixture.assert_guarded(&current, None);
        }
    }
}

#[tokio::test]
async fn ensure_rechecks_scope_after_refreshing_current_team_credit() {
    for dimension in ["user", "device", "team", "project", "proxy", "removed"] {
        let mut current = account(1);
        let mut newer = current.clone();
        if dimension != "removed" { change_scope(&mut newer, dimension); }
        let replacement = if dimension == "removed" { None } else { Some(newer) };
        let fixture = Fixture::new(&current);
        fixture.change_at(Boundary::CurrentUsage, replacement.clone());
        assert!(matches!(fixture.run(Entry::Ensure, &mut current).await,
            Err(CoreError::NotEligible(_))));
        fixture.assert_guarded(&current, replacement.as_ref());
        fixture.finish(0);
    }
}

#[tokio::test]
async fn mismatched_session_refuses_before_credit_lookup_or_publication() {
    for entry in ENTRIES {
        for available in [0.0, 80.0] {
            let mut current = account(1);
            current.credit = Some(credit(available));
            let fixture = Fixture::new(&current);
            let session = Session {
                active_user: fixture.other.framer_user_id.clone(),
                cookie_header: format!("session=inert-{}", fixture.other.framer_user_id.0),
            };
            let deps = Deps { client: &fixture, browser: &fixture, sessions: &fixture,
                state: &fixture, account_updates: &fixture.updates };
            let result = match entry {
                Entry::Ensure => ensure_workspace(&deps, &mut current, &session).await,
                Entry::Rotate => rotate_workspace(&deps, &mut current, &session).await,
            };
            assert!(matches!(result, Err(CoreError::NotEligible(_))),
                "mismatched owner, {entry:?}, available={available}");
            fixture.assert_unchanged(&current);
            assert!(fixture.writes.lock().unwrap().is_empty());
            fixture.finish(0);
        }
    }
}
