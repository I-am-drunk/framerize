use super::*;
use std::time::Duration;
const WAIT: Duration = Duration::from_millis(2);

#[tokio::test]
async fn named_application_completion_does_not_release_retained_host() {
    let (gate, _host) = OperationGate::new_with_legacy_host(3).unwrap();
    let first = gate.admit(OperationKind::Admin).unwrap();
    let second = gate.admit(OperationKind::ExternalCleanup).unwrap();
    assert_ne!(first.id, second.id);
    let mut ticket = gate.seal().unwrap();
    first.complete().unwrap();
    assert!(tokio::time::timeout(WAIT, ticket.wait_application()).await.is_err());
    second.complete().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    gate.check_application_drained(&proof).unwrap();
    drop(proof);
    assert!(tokio::time::timeout(WAIT, ticket.wait()).await.is_err());
    assert!(gate.stats().unwrap().sealed);
}

#[tokio::test]
async fn application_drain_includes_children_registered_after_seal() {
    let (gate, _host) = OperationGate::new_with_legacy_host(3).unwrap();
    let parent = gate.admit(OperationKind::Inference).unwrap();
    let mut ticket = gate.seal().unwrap();
    let child = parent.register_cleanup().unwrap();
    parent.complete().unwrap();
    let mut draining = std::pin::pin!(ticket.wait_application());
    assert!(futures::poll!(draining.as_mut()).is_pending());
    child.complete().unwrap();
    gate.check_application_drained(&draining.await.unwrap()).unwrap();
}

#[tokio::test]
async fn application_timeout_and_precontrol_cancel_preserve_original_owner() {
    let (gate, _host) = OperationGate::new_with_legacy_host(3).unwrap();
    let owner = gate.admit(OperationKind::SessionWrite).unwrap();
    let id = owner.id;
    let mut ticket = gate.seal().unwrap();
    assert!(tokio::time::timeout(WAIT, ticket.wait_application()).await.is_err());
    assert_eq!(gate.inner.state.lock().unwrap().applications.get(&id),
        Some(&OperationKind::SessionWrite));
    ticket.cancel_before_control().unwrap();
    let next = gate.admit(OperationKind::StateWrite).unwrap();
    assert_ne!(id, next.id);
    owner.complete().unwrap(); next.complete().unwrap();
    let mut ticket = gate.seal().unwrap();
    gate.check_application_drained(&ticket.wait_application().await.unwrap()).unwrap();
}

#[tokio::test]
async fn abandoned_application_prevents_observation_with_empty_registry() {
    let (gate, _host) = OperationGate::new_with_legacy_host(2).unwrap();
    let owner = gate.admit(OperationKind::Authorization).unwrap();
    let mut ticket = gate.seal().unwrap();
    drop(owner);
    assert!(gate.inner.state.lock().unwrap().applications.is_empty());
    assert!(matches!(ticket.wait_application().await, Err(GateError::Unconfirmed)));
    assert!(matches!(ticket.wait().await, Err(GateError::Unconfirmed)));
    assert_eq!(ticket.cancel_before_control().err(), Some(GateError::Unconfirmed));
}

#[tokio::test]
async fn application_observation_rejects_other_gate_and_stale_seal() {
    let (gate, _host) = OperationGate::new_with_legacy_host(1).unwrap();
    let (other, _other_host) = OperationGate::new_with_legacy_host(1).unwrap();
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    assert_eq!(other.check_application_drained(&proof), Err(GateError::WrongGate));
    // Safe callers cannot cancel the borrowed ticket. Inject a changed seal
    // privately to check the defensive consumer validation independently.
    gate.inner.state.lock().unwrap().phase = Phase::Draining(proof.ticket.id + 1);
    assert_eq!(gate.check_application_drained(&proof), Err(GateError::StaleSeal));
}

#[tokio::test]
async fn abandoned_host_invalidates_application_observation_and_full_drain() {
    let (gate, host) = OperationGate::new_with_legacy_host(1).unwrap();
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    drop(host);
    assert_eq!(gate.check_application_drained(&proof), Err(GateError::Unconfirmed));
    drop(proof);
    assert!(matches!(ticket.wait().await, Err(GateError::Unconfirmed)));
    assert!(gate.inner.state.lock().unwrap().legacy_host == LegacyHostState::Abandoned);
}

#[tokio::test]
async fn hostless_gate_does_not_issue_retained_host_application_observation() {
    let gate = OperationGate::new(1).unwrap();
    let mut ticket = gate.seal().unwrap();
    assert!(matches!(ticket.wait_application().await, Err(GateError::NotDrained)));
    let full = ticket.wait().await.unwrap();
    gate.check_held(&full).unwrap();
}

#[tokio::test]
async fn poisoned_gate_never_issues_or_accepts_application_observation() {
    let (gate, _host) = OperationGate::new_with_legacy_host(1).unwrap();
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lock = gate.inner.state.lock().unwrap(); panic!("fixture gate poison");
    }));
    assert_eq!(gate.check_application_drained(&proof), Err(GateError::Poisoned));
    drop(proof);
    assert!(matches!(ticket.wait_application().await, Err(GateError::Poisoned)));
}

#[test]
fn named_application_ids_are_bounded_and_never_reused() {
    let (gate, _host) = OperationGate::new_with_legacy_host(2).unwrap();
    let first = gate.admit(OperationKind::Admin).unwrap();
    let id = first.id;
    assert!(matches!(first.register_cleanup(), Err(GateError::Full)));
    first.complete().unwrap();
    let second = gate.admit(OperationKind::Admin).unwrap();
    assert_ne!(id, second.id);
    second.complete().unwrap();
    gate.inner.state.lock().unwrap().next_operation = u64::MAX;
    assert!(matches!(gate.admit(OperationKind::Admin), Err(GateError::Exhausted)));
    assert!(gate.inner.state.lock().unwrap().applications.is_empty());
}
