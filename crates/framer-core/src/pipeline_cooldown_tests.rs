//! Exercise pipeline::run through inert, in-memory public port implementations.
//! No browser, network, sleeps, SDK, tasks or real account data are used.
use super::{run, Deps, InferenceOutcome, InferenceRequest, InferenceResult, Usage};
use crate::account::{AccountState, ManagedAccount, WorkspaceScope};
use crate::convert::{AssembledResponse, AssembledResponses, ChainEvent};
use crate::error::CoreError;
use crate::ids::{AccountId, FramerUserId, ProjectId, TeamId};
use crate::port::*;
use crate::workspace::CreditSnapshot;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Mutex;

enum Change { Replace(ManagedAccount), Remove(AccountId) }
struct Reply {
    result: Result<InferenceResult, String>,
    change: Option<Change>,
}

struct Fixture {
    records: Mutex<Vec<ManagedAccount>>,
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<&'static str>>,
    updates: tokio::sync::Mutex<()>,
}

fn account(index: u8) -> ManagedAccount {
    ManagedAccount {
        id: AccountId(uuid::Uuid::from_u128(0xCD00 + u128::from(index))),
        alias: format!("cooldown-fixture-{index}"), email: None,
        framer_user_id: FramerUserId(format!("cooldown-user-{index}")),
        castle_device_id: format!("cooldown-device-{index}"), proxy: None,
        current_team: Some(TeamId(format!("cooldown-team-{index}"))),
        current_project: Some(ProjectId(format!("cooldown-project-{index}"))),
        extra_workspaces: if index == 1 { vec![WorkspaceScope {
            team: TeamId("cooldown-rotated-team".into()),
            project: ProjectId("cooldown-rotated-project".into()),
        }] } else { vec![] },
        state: AccountState::Ready,
        credit: Some(credit()), consecutive_failures: 0,
        cooldown_step: 0, cooling_until_ms: None, cooldown_cause: None,
    }
}

fn credit() -> CreditSnapshot {
    CreditSnapshot { available: 100.0, funds: 1, grants: 1, allocated: 100.0 }
}

fn reply(result: Result<InferenceResult, String>) -> Reply {
    Reply { result, change: None }
}

fn completed() -> InferenceResult {
    InferenceResult::Completed(InferenceOutcome {
        text: "inert response".into(), usage: Usage::default(),
        chat_session: "inert-chat".into(), credits_remaining: Some(100.0),
    })
}

fn exhausted() -> InferenceResult {
    InferenceResult::Exhausted { reason: "inert 402".into() }
}

impl Fixture {
    fn new(current: &ManagedAccount, replies: Vec<Reply>) -> Self {
        Self {
            records: Mutex::new(vec![current.clone(), account(2)]),
            replies: Mutex::new(replies.into()), calls: Mutex::new(vec![]),
            updates: tokio::sync::Mutex::new(()),
        }
    }

    fn record(&self, call: &'static str) { self.calls.lock().unwrap().push(call); }

    fn stored(&self, id: AccountId) -> Option<ManagedAccount> {
        self.records.lock().unwrap().iter().find(|a| a.id == id).cloned()
    }

    async fn run(&self, current: &mut ManagedAccount) -> Result<InferenceOutcome, CoreError> {
        let deps = Deps { client: self, browser: self, sessions: self,
            state: self, account_updates: &self.updates };
        run(&deps, current, &InferenceRequest {
            prompt: "inert cooldown fixture".into(), model: None, chat_session: None,
        }).await
    }

    fn assert_finished(&self, expected_calls: &[&str]) {
        assert_eq!(self.calls.lock().unwrap().as_slice(), expected_calls);
        assert!(self.replies.lock().unwrap().is_empty());
        assert_record_eq(&self.stored(account(2).id).unwrap(), &account(2));
    }
}

fn assert_record_eq(actual: &ManagedAccount, expected: &ManagedAccount) {
    assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(expected).unwrap());
}

#[async_trait]
impl StateStore for Fixture {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        Ok(self.stored(id))
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        Ok(self.records.lock().unwrap().clone())
    }
    async fn upsert(&self, current: &ManagedAccount) -> Result<(), String> {
        self.record("upsert");
        let mut records = self.records.lock().unwrap();
        records.retain(|a| a.id != current.id);
        records.push(current.clone());
        Ok(())
    }
    async fn remove(&self, _id: AccountId) -> Result<(), String> {
        panic!("pipeline unexpectedly removed an account")
    }
}

#[async_trait]
impl SessionStore for Fixture {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        self.record("load");
        assert_eq!(user, &account(1).framer_user_id);
        Ok(Some(Session { active_user: user.clone(), cookie_header: "session=inert".into() }))
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

fn assert_session(session: &Session) {
    assert_eq!(session.active_user, account(1).framer_user_id);
    assert_eq!(session.cookie_header, "session=inert");
}

#[async_trait]
impl FramerClient for Fixture {
    async fn run_inference(&self, session: &Session, _: &dyn BrowserBridge,
        id: &AccountId, project: &ProjectId, request: &InferenceRequest)
        -> Result<InferenceResult, String> {
        self.record(match project.0.as_str() {
            "cooldown-project-1" => "run:initial",
            "cooldown-rotated-project" => "run:rotated",
            _ => panic!("inference used an unexpected fixture project"),
        });
        assert_session(session);
        assert_eq!(*id, account(1).id);
        assert_eq!(Some(project), self.stored(*id).unwrap().current_project.as_ref());
        assert_eq!(request.prompt, "inert cooldown fixture");
        assert!(request.model.is_none() && request.chat_session.is_none());
        let next = self.replies.lock().unwrap().pop_front().expect("unexpected extra inference");
        if let Some(change) = next.change {
            // Model a completed concurrent write at the response boundary under the shared lock.
            let _update = self.updates.lock().await;
            let mut records = self.records.lock().unwrap();
            match change {
                Change::Replace(record) => {
                    assert_eq!(record.id, *id);
                    records.retain(|a| a.id != record.id);
                    records.push(record);
                }
                Change::Remove(removed) => {
                    assert_eq!(removed, *id);
                    records.retain(|a| a.id != removed);
                }
            }
        }
        next.result
    }

    async fn me(&self, session: &Session) -> Result<Value, String> {
        self.record("me");
        assert_session(session);
        Ok(json!({"spaces": [{"ownerId": "cooldown-rotated-team"}]}))
    }
    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        self.record("usage");
        assert_session(session);
        assert_eq!(team.0, "cooldown-rotated-team");
        Ok(credit())
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

fn script(rotated: bool, last: Reply) -> Vec<Reply> {
    if rotated { vec![reply(Ok(exhausted())), last] } else { vec![last] }
}

fn expected_calls(rotated: bool, final_write: bool) -> Vec<&'static str> {
    let mut calls = if rotated {
        vec!["load", "run:initial", "me", "usage", "upsert", "run:rotated"]
    } else { vec!["load", "run:initial"] };
    if final_write { calls.push("upsert"); }
    calls
}

fn operation_scope(rotated: bool) -> ManagedAccount {
    let mut current = account(1);
    if rotated {
        current.current_team = Some(TeamId("cooldown-rotated-team".into()));
        current.current_project = Some(ProjectId("cooldown-rotated-project".into()));
        current.extra_workspaces.clear();
    }
    current
}

fn assert_risk(result: Result<InferenceOutcome, CoreError>) {
    assert!(matches!(result, Err(CoreError::RiskRefused(ref cause)) if cause == "risk-refused"));
}

fn assert_cooling_between(result: Result<InferenceOutcome, CoreError>,
    until: u64, before: u64, after: u64) {
    let Err(CoreError::AccountCooling(cooldown, _)) = result else {
        panic!("expected typed local cooldown");
    };
    let minimum = until.saturating_sub(after).div_ceil(1000).max(1);
    let maximum = until.saturating_sub(before).div_ceil(1000);
    assert!((minimum..=maximum).contains(&cooldown.retry_after_secs));
    assert!(cooldown.retry_after_secs > 30, "core must retain the full deadline");
}

fn assert_window(current: &ManagedAccount, start: u64, end: u64, prior_step: u32) {
    let until = current.cooling_until_ms.expect("risk refusal must record an expiry");
    let backoff = ManagedAccount::cooldown_backoff_ms(prior_step);
    assert!((start + backoff..=end + backoff).contains(&until));
    assert_eq!(current.state, AccountState::Ready);
    assert!(current.is_eligible());
    assert!(!current.is_available_at(start));
    assert!(!current.is_available_at(until - 1));
    assert!(current.is_available_at(until));
}

#[tokio::test]
async fn risk_refused_keeps_ready_account_eligible_until_deadline_expires() {
    let mut current = account(1);
    let fixture = Fixture::new(&current, vec![reply(Err("risk-refused".into()))]);
    let start = crate::rotation::now_ms();
    assert_risk(fixture.run(&mut current).await);
    assert_window(&current, start, crate::rotation::now_ms(), 0);
    assert_eq!(current.consecutive_failures, 1);
    assert_eq!(current.cooldown_step, 1);
    assert_record_eq(&fixture.stored(current.id).unwrap(), &current);
    fixture.assert_finished(&expected_calls(false, true));
}

#[tokio::test]
async fn risk_refused_after_402_rotation_uses_same_cooldown_without_third_attempt() {
    let mut current = account(1);
    current.cooldown_step = 2;
    current.consecutive_failures = 4;
    let fixture = Fixture::new(&current, script(true, reply(Err("risk-refused".into()))));
    let start = crate::rotation::now_ms();
    assert_risk(fixture.run(&mut current).await);
    assert_window(&current, start, crate::rotation::now_ms(), 2);
    assert_eq!(current.current_team, operation_scope(true).current_team);
    assert_eq!(current.current_project, operation_scope(true).current_project);
    assert!(current.extra_workspaces.is_empty());
    assert_eq!(current.cooldown_step, 3);
    assert_eq!(current.consecutive_failures, 5);
    assert_record_eq(&fixture.stored(current.id).unwrap(), &current);
    fixture.assert_finished(&expected_calls(true, true));
}

#[tokio::test]
async fn cooldown_recorded_during_first_inference_prevents_402_retry() {
    let mut current = account(1);
    let mut newer = current.clone();
    newer.cooling_until_ms = Some(crate::rotation::now_ms() + 3_600_000);
    newer.cooldown_step = 3;
    newer.consecutive_failures = 7;
    newer.alias = "newer cooldown label".into();
    let first = Reply { result: Ok(exhausted()),
        change: Some(Change::Replace(newer.clone())) };
    let fixture = Fixture::new(&current, vec![first]);
    let mut expected = newer;
    expected.current_team = operation_scope(true).current_team;
    expected.current_project = operation_scope(true).current_project;
    expected.extra_workspaces.clear();
    let before = crate::rotation::now_ms();
    let result = fixture.run(&mut current).await;
    assert_cooling_between(result, expected.cooling_until_ms.unwrap(),
        before, crate::rotation::now_ms());
    assert_record_eq(&current, &expected);
    assert_record_eq(&fixture.stored(current.id).unwrap(), &expected);
    assert!(current.is_eligible());
    assert!(!current.is_available_at(crate::rotation::now_ms()));
    assert!(current.is_available_at(current.cooling_until_ms.unwrap()));
    fixture.assert_finished(&["load", "run:initial", "me", "usage", "upsert"]);
}

#[tokio::test]
async fn nonready_recorded_during_first_inference_prevents_402_retry() {
    let mut current = account(1);
    let mut newer = current.clone();
    newer.state = AccountState::Quarantined;
    newer.cooldown_step = 2;
    newer.consecutive_failures = 4;
    newer.alias = "newer lifecycle label".into();
    let first = Reply { result: Ok(exhausted()),
        change: Some(Change::Replace(newer.clone())) };
    let fixture = Fixture::new(&current, vec![first]);
    let mut expected = newer;
    expected.current_team = operation_scope(true).current_team;
    expected.current_project = operation_scope(true).current_project;
    expected.extra_workspaces.clear();
    assert!(matches!(fixture.run(&mut current).await, Err(CoreError::AccountNotReady(_))));
    assert_record_eq(&current, &expected);
    assert_record_eq(&fixture.stored(current.id).unwrap(), &expected);
    assert!(!current.is_eligible());
    assert!(!current.is_available_at(crate::rotation::now_ms()));
    fixture.assert_finished(&["load", "run:initial", "me", "usage", "upsert"]);
}

#[tokio::test]
async fn initial_inference_capacity_refusal_has_typed_cause() {
    for kind in ["cooldown", "nonready", "both"] {
        let mut current = account(1);
        let mut newer = current.clone();
        if kind != "nonready" { newer.cooling_until_ms = Some(u64::MAX); }
        if kind != "cooldown" { newer.state = AccountState::Quarantined; }
        let fixture = Fixture::new(&newer, vec![]);
        let before = crate::rotation::now_ms();
        let result = fixture.run(&mut current).await;
        if kind == "nonready" {
            assert!(matches!(result, Err(CoreError::AccountNotReady(_))));
        } else {
            assert_cooling_between(result, u64::MAX, before, crate::rotation::now_ms());
        }
        assert_record_eq(&current, &account(1));
        assert_record_eq(&fixture.stored(current.id).unwrap(), &newer);
        fixture.assert_finished(&["load"]);
    }
}

#[tokio::test]
async fn invalid_admission_scope_is_not_typed_capacity() {
    for dimension in ["removed", "user", "device", "team", "project", "proxy",
        "session", "missing-team", "missing-project"] {
        let mut current = account(1);
        let mut newer = current.clone();
        newer.state = AccountState::Quarantined;
        newer.cooling_until_ms = Some(u64::MAX);
        let mut session = Session { active_user: current.framer_user_id.clone(),
            cookie_header: "session=inert".into() };
        match dimension {
            "user" => newer.framer_user_id = FramerUserId("changed-user".into()),
            "device" => newer.castle_device_id = "changed-device".into(),
            "team" => newer.current_team = Some(TeamId("changed-team".into())),
            "project" => newer.current_project = Some(ProjectId("changed-project".into())),
            "proxy" => newer.proxy = Some("http://changed.invalid:8080".into()),
            "session" => session.active_user = FramerUserId("changed-user".into()),
            "missing-team" => { current.current_team = None; newer.current_team = None; }
            "missing-project" => { current.current_project = None; newer.current_project = None; }
            "removed" => {}
            _ => unreachable!(),
        }
        let fixture = Fixture::new(&newer, vec![]);
        if dimension == "removed" {
            fixture.records.lock().unwrap().retain(|a| a.id != current.id);
        }
        let deps = Deps { client: &fixture, browser: &fixture, sessions: &fixture,
            state: &fixture, account_updates: &fixture.updates };
        let expected = match dimension {
            "removed" => "account removed before inference",
            "missing-team" => "account state is Quarantined; inference requires Ready; no current workspace is configured",
            "missing-project" => "account state is Quarantined; inference requires Ready; no current project is configured",
            _ => "account scope changed before inference",
        };
        assert!(matches!(super::check_inference_admission(&deps, &current, &session).await,
            Err(CoreError::NotEligible(ref cause)) if cause == expected), "{dimension}");
        if dimension == "removed" { assert!(fixture.stored(current.id).is_none()); }
        else { assert_record_eq(&fixture.stored(current.id).unwrap(), &newer); }
        fixture.assert_finished(&[]);
    }
}

#[tokio::test]
async fn ready_record_replacement_preserves_one_successful_402_retry() {
    let mut current = account(1);
    let mut newer = current.clone();
    newer.cooling_until_ms = Some(42); // Expired cooldown remains admissible.
    newer.cooldown_step = 3;
    newer.consecutive_failures = 7;
    newer.alias = "newer ready label".into();
    let first = Reply { result: Ok(exhausted()),
        change: Some(Change::Replace(newer.clone())) };
    let fixture = Fixture::new(&current, vec![first, reply(Ok(completed()))]);
    let mut expected = newer;
    expected.current_team = operation_scope(true).current_team;
    expected.current_project = operation_scope(true).current_project;
    expected.extra_workspaces.clear();
    let outcome = fixture.run(&mut current).await.expect("ready retry must complete");
    assert_eq!(outcome.text, "inert response");
    assert_eq!(outcome.chat_session, "inert-chat");
    assert_eq!(outcome.credits_remaining, Some(100.0));
    assert_record_eq(&current, &expected);
    assert_record_eq(&fixture.stored(current.id).unwrap(), &expected);
    assert!(current.is_available_at(crate::rotation::now_ms()));
    fixture.assert_finished(&expected_calls(true, false));
}

#[tokio::test]
async fn risk_refused_preserves_longer_current_deadline_and_newer_fields() {
    for rotated in [false, true] {
        let mut current = account(1);
        let mut newer = operation_scope(rotated);
        newer.cooling_until_ms = Some(crate::rotation::now_ms() + 3_600_000);
        newer.cooldown_step = 3;
        newer.consecutive_failures = 7;
        newer.alias = "newer account label".into();
        newer.email = Some("newer@example.test".into());
        newer.credit.as_mut().unwrap().available = 73.0;
        newer.extra_workspaces.push(WorkspaceScope {
            team: TeamId("extra-team".into()), project: ProjectId("extra-project".into()),
        });
        let last = Reply { result: Err("risk-refused".into()),
            change: Some(Change::Replace(newer.clone())) };
        let fixture = Fixture::new(&current, script(rotated, last));
        assert_risk(fixture.run(&mut current).await);
        newer.cooldown_step = 4;
        newer.consecutive_failures = 8;
        assert_record_eq(&current, &newer);
        assert_record_eq(&fixture.stored(current.id).unwrap(), &newer);
        let until = newer.cooling_until_ms.unwrap();
        assert!(current.is_eligible());
        assert!(!current.is_available_at(until - 1));
        assert!(current.is_available_at(until));
        fixture.assert_finished(&expected_calls(rotated, true));
    }
}

#[tokio::test]
async fn risk_refused_does_not_revive_nonready_current_accounts() {
    for state in [AccountState::Disconnected, AccountState::Authenticating,
        AccountState::CoolingDown, AccountState::Exhausted, AccountState::Quarantined] {
        for rotated in [false, true] {
            let mut current = account(1);
            let mut newer = operation_scope(rotated);
            newer.state = state.clone();
            let last = Reply { result: Err("risk-refused".into()),
                change: Some(Change::Replace(newer.clone())) };
            let fixture = Fixture::new(&current, script(rotated, last));
            assert_risk(fixture.run(&mut current).await);
            newer.cooldown_step = 1;
            newer.consecutive_failures = 1;
            newer.cooling_until_ms = Some(current.cooling_until_ms.expect("recorded expiry"));
            newer.cooldown_cause = Some("Framer returned HTTP 403 during its security checks".into());
            assert_record_eq(&current, &newer);
            assert_record_eq(&fixture.stored(current.id).unwrap(), &newer);
            assert!(!current.is_eligible());
            assert!(!current.is_available_at(newer.cooling_until_ms.unwrap()));
            fixture.assert_finished(&expected_calls(rotated, true));
        }
    }
}

#[tokio::test]
async fn risk_refused_rejects_changed_current_account_scope() {
    for dimension in ["user", "device", "team", "project", "proxy"] {
        for rotated in [false, true] {
            let mut current = account(1);
            let mut replacement = operation_scope(rotated);
            match dimension {
                "user" => replacement.framer_user_id = FramerUserId("changed-user".into()),
                "device" => replacement.castle_device_id = "changed-device".into(),
                "team" => replacement.current_team = Some(TeamId("changed-team".into())),
                "project" => replacement.current_project = Some(ProjectId("changed-project".into())),
                "proxy" => replacement.proxy = Some("http://changed.invalid:8080".into()),
                _ => unreachable!(),
            }
            let last = Reply { result: Err("risk-refused".into()),
                change: Some(Change::Replace(replacement.clone())) };
            let fixture = Fixture::new(&current, script(rotated, last));
            assert!(matches!(fixture.run(&mut current).await, Err(CoreError::NotEligible(_))),
                "changed {dimension}, rotated={rotated}");
            assert_record_eq(&fixture.stored(current.id).unwrap(), &replacement);
            assert_record_eq(&current, &operation_scope(rotated));
            fixture.assert_finished(&expected_calls(rotated, false));
        }
    }
}

#[tokio::test]
async fn risk_refused_does_not_recreate_removed_accounts() {
    for rotated in [false, true] {
        let mut current = account(1);
        let last = Reply { result: Err("risk-refused".into()),
            change: Some(Change::Remove(current.id)) };
        let fixture = Fixture::new(&current, script(rotated, last));
        assert!(matches!(fixture.run(&mut current).await, Err(CoreError::NotEligible(_))));
        assert!(fixture.stored(current.id).is_none());
        assert_record_eq(&current, &operation_scope(rotated));
        fixture.assert_finished(&expected_calls(rotated, false));
    }
}

#[tokio::test]
async fn non_risk_errors_do_not_add_or_clear_risk_cooldown() {
    for cause in ["risk-refused ", "wrapped risk-refused", "Risk-refused",
        "upstream 403", "account-cooling:60", "upstream 429", "retryable 500"] {
        for rotated in [false, true] {
            for deadline in [None, Some(42)] {
                let mut current = account(1);
                current.cooling_until_ms = deadline;
                current.cooldown_step = 3;
                current.consecutive_failures = 4;
                let mut expected = current.clone();
                expected.current_team = operation_scope(rotated).current_team;
                expected.current_project = operation_scope(rotated).current_project;
                expected.extra_workspaces = operation_scope(rotated).extra_workspaces;
                let fixture = Fixture::new(&current, script(rotated, reply(Err(cause.into()))));
                assert!(matches!(fixture.run(&mut current).await,
                    Err(CoreError::Framer(ref message)) if message == cause));
                assert_record_eq(&current, &expected);
                assert_record_eq(&fixture.stored(current.id).unwrap(), &expected);
                fixture.assert_finished(&expected_calls(rotated, false));
            }
        }
    }
}

#[tokio::test]
async fn completed_results_do_not_add_risk_cooldown() {
    for rotated in [false, true] {
        let mut current = account(1);
        let fixture = Fixture::new(&current, script(rotated, reply(Ok(completed()))));
        let outcome = fixture.run(&mut current).await.expect("inert completion");
        assert_eq!(outcome.text, "inert response");
        assert_eq!(outcome.chat_session, "inert-chat");
        assert_eq!(outcome.credits_remaining, Some(100.0));
        assert_record_eq(&current, &operation_scope(rotated));
        assert_record_eq(&fixture.stored(current.id).unwrap(), &current);
        fixture.assert_finished(&expected_calls(rotated, false));
    }
}

#[tokio::test]
async fn repeated_exhaustion_marks_exhausted_without_risk_cooldown_or_third_attempt() {
    let mut current = account(1);
    let fixture = Fixture::new(&current, script(true, reply(Ok(exhausted()))));
    assert!(matches!(fixture.run(&mut current).await, Err(CoreError::NotEligible(_))));
    let mut expected = operation_scope(true);
    expected.state = AccountState::Exhausted;
    assert_record_eq(&current, &expected);
    assert_record_eq(&fixture.stored(current.id).unwrap(), &expected);
    fixture.assert_finished(&expected_calls(true, true));
}
