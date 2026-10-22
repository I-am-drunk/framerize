use super::*;
use crate::native_control::{admit, AdmittedControl};
use framer_core::native_dispatch::{FailureKind, Identity};
use tokio::time::Instant;

#[path = "native_update_tests.rs"]
mod updates;

#[path = "native_routing_tests.rs"]
mod routing;

#[tokio::test]
async fn legacy_health_publication_preserves_newer_account_fields() {
    let (state, ports) = owned();
    let stale = account(1);
    ports.records.lock().unwrap()[0].alias = "updated during inference".into();
    let failure: Result<(), String> = Err("risk-refused".into());
    crate::record_account_health(&state, &stale, &failure).await;
    let current = state.state.get(id(1)).await.unwrap().unwrap();
    assert_eq!(current.alias, "updated during inference");
    assert_eq!(current.consecutive_failures, stale.consecutive_failures + 1);
}

#[tokio::test]
async fn exhausted_control_gate_rejects_before_browser_io() {
    let (state, ports) = fixture(2, 2);
    // One slot is permanently occupied by the legacy host, one by this job.
    let state = crate::operation_ownership::with_owned_operations(state, 2).unwrap();
    let (release, wait) = tokio::sync::oneshot::channel::<()>();
    let task = crate::operation_ownership::spawn(state.operation_ownership.as_ref(),
        framer_core::operation_gate::OperationKind::Inference,
        async move { let _ = wait.await; }).unwrap();
    assert_eq!(acquire(&state).await.err().unwrap().kind, FailureKind::Ineligible);
    assert!(ports.events.lock().unwrap().is_empty());
    release.send(()).unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn sealed_control_rejects_before_status_or_victim_close() {
    let (state, ports) = owned();
    let seal = state.operation_ownership.as_ref().unwrap().seal().unwrap();
    assert_eq!(acquire(&state).await.err().unwrap().kind, FailureKind::Ineligible);
    assert!(ports.events.lock().unwrap().is_empty());
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
    drop(seal);
}

#[tokio::test]
async fn control_model_wait_cannot_outlive_a_lifecycle_fence() {
    let (state, ports) = owned();
    let (held, task) = waiting(&state).await;
    let observer = lease_account(&state, id(1)).await.unwrap();
    observer.mark_unconfirmed().unwrap();
    drop(observer);
    drop(held);
    assert_eq!(task.await.unwrap().err().unwrap().kind, FailureKind::Ineligible);
    assert_eq!(slot(&state, id(1)).users, 0);
    assert!(slot(&state, id(1)).uncertain);
    assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1)), ("status", id(1))]);
}

#[tokio::test]
async fn control_invalid_deadline_rejects_without_external_work() {
    let (state, ports) = owned();
    let result = admit(&state, identity(), "gpt-4.1", Instant::now()).await;
    assert_eq!(result.err().unwrap().kind, FailureKind::InvalidRequest);
    assert!(ports.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn control_without_serving_ownership_rejects_before_browser_or_session_io() {
    let (state, ports) = fixture(2, 2);
    assert_eq!(acquire(&state).await.err().unwrap().kind, FailureKind::Ineligible);
    assert!(ports.events.lock().unwrap().is_empty());
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
}

#[tokio::test]
async fn control_rejects_changed_identity_before_admission() {
    for change in 0..3 {
        let (state, ports) = owned();
        let mut selected = identity();
        match change { 0 => selected.user.0.push('x'),
            1 => selected.team.0.push('x'), _ => selected.project.0.push('x') }
        let result = admit(&state, selected, "gpt-4.1",
            Instant::now() + Duration::from_secs(3)).await;
        assert_eq!(result.err().unwrap().kind, FailureKind::InvalidScope);
        assert!(ports.events.lock().unwrap().is_empty());
    }
}

fn identity() -> Identity {
    let a = account(1);
    Identity { account: a.id, user: a.framer_user_id,
        team: a.current_team.unwrap(), project: a.current_project.unwrap(),
        chat_session: "actual-test-conversation".into() }
}
fn owned() -> (AppState, Arc<Ports>) {
    let (mut state, ports) = fixture(2, 2);
    state.gate = one_slot_gate();
    (crate::operation_ownership::with_owned_operations(state, 32).unwrap(), ports)
}

async fn waiting(state: &AppState)
    -> (tokio::sync::OwnedSemaphorePermit,
        tokio::task::JoinHandle<Result<AdmittedControl, framer_core::native_dispatch::DispatchError>>) {
    let held = state.gate.acquire("team-1:openai/gpt-4.1").await;
    let task_state = state.clone();
    let task = tokio::spawn(async move { acquire(&task_state).await });
    until(|| state.serving_accounts.residents.snapshot().unwrap().residents
        .iter().any(|slot| slot.account == id(1) && slot.users > 0)).await;
    (held, task)
}

#[tokio::test]
async fn control_cancelled_model_wait_retains_actual_admission_until_completion() {
    let (state, ports) = owned();
    let (held, task) = waiting(&state).await;
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    assert_eq!(slot(&state, id(1)).users, 1);
    assert!(!slot(&state, id(1)).uncertain);
    assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1))]);
    drop(held);
    until(|| slot(&state, id(1)).users == 0).await;
    drop(acquire(&state).await.unwrap());
}

#[tokio::test]
async fn control_model_wait_rechecks_identity_proxy_and_wave_without_repicking() {
    for change in 0..3 {
        let (state, ports) = owned();
        let (held, task) = waiting(&state).await;
        match change {
            0 => ports.records.lock().unwrap()[0].current_project = Some(ProjectId("changed".into())),
            1 => ports.records.lock().unwrap()[0].proxy = None,
            _ => { state.wave.note_refusal("one"); state.wave.note_refusal("two"); }
        }
        drop(held);
        let error = task.await.unwrap().err().unwrap();
        assert_eq!(error.kind, if change == 2 { FailureKind::Overloaded }
            else { FailureKind::InvalidScope });
        assert_eq!(slot(&state, id(1)).users, 0);
        assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1))]);
    }
}
async fn acquire(state: &AppState) -> Result<AdmittedControl, framer_core::native_dispatch::DispatchError> {
    admit(state, identity(), "gpt-4.1", Instant::now() + Duration::from_secs(3)).await
}

#[tokio::test]
async fn control_admission_uses_existing_pool_and_blocks_account_mutation() {
    let (state, ports) = owned();
    let admitted = acquire(&state).await.unwrap();
    assert_eq!(admitted.identity, identity());
    assert_eq!(admitted.proxy, account(1).proxy);
    assert_eq!(admitted.pool.account(), id(1));
    assert_eq!(slot(&state, id(1)).users, 2);
    assert!(management(&state, id(1), false).await.is_err());
    assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1))]);
    drop(admitted);
    assert_eq!(slot(&state, id(1)).users, 0);
    assert!(management(&state, id(1), false).await.is_ok());
}

#[tokio::test]
async fn control_session_owner_outlives_plan_pool_cleanup() {
    let (state, _) = owned();
    let mut admitted = acquire(&state).await.unwrap();
    admitted.completion.start();
    admitted.pool.complete().unwrap();
    drop(admitted.capacity);
    assert_eq!(slot(&state, id(1)).users, 1);
    assert!(management(&state, id(1), false).await.is_err());
    admitted.completion.complete().unwrap();
    assert_eq!(slot(&state, id(1)).users, 0);
    assert!(management(&state, id(1), false).await.is_ok());
}

#[tokio::test]
async fn abandoned_control_session_retains_unconfirmed_account() {
    let (state, _) = owned();
    let mut admitted = acquire(&state).await.unwrap();
    admitted.completion.start();
    admitted.pool.complete().unwrap();
    drop(admitted.completion);
    drop(admitted.capacity);
    assert_eq!(slot(&state, id(1)).users, 1);
    assert!(slot(&state, id(1)).uncertain);
    assert!(management(&state, id(1), false).await.is_err());
}

#[tokio::test]
async fn control_outcomes_preserve_fresh_fields_cooldown_and_wave() {
    use crate::native_control::HealthOutcome;
    let (state, ports) = owned();
    let mut admitted = acquire(&state).await.unwrap();
    admitted.completion.start();
    let until = framer_core::rotation::now_ms() + 600_000;
    {
        let mut rows = ports.records.lock().unwrap();
        rows[0].alias = "fresh alias".into();
        rows[0].cooling_until_ms = Some(until);
        rows[0].cooldown_step = 3;
    }
    state.wave.note_refusal("first"); state.wave.note_refusal("second");
    for outcome in [HealthOutcome::Unclassified, HealthOutcome::Complete] {
        admitted.completion.publish(&state, &admitted.identity, &admitted.proxy, outcome).await.unwrap();
    }
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
    assert!(state.wave.check().is_some());
    admitted.completion.publish(&state, &admitted.identity, &admitted.proxy,
        HealthOutcome::RiskRefused).await.unwrap();
    let fresh = state.state.get(id(1)).await.unwrap().unwrap();
    assert_eq!(fresh.alias, "fresh alias");
    assert_eq!(fresh.cooling_until_ms, Some(until));
    assert_eq!(fresh.cooldown_step, 4);
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), 1);
    admitted.pool.complete().unwrap();
    admitted.completion.complete().unwrap();
}
