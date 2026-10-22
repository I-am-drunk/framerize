use super::*;
use std::time::Duration;

#[tokio::test]
async fn retirement_is_irreversible_without_host_completion() {
    let (gate, _host) = OperationGate::new_with_legacy_host(2).unwrap();
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    let retirement = gate.begin_host_retirement(&proof).unwrap();
    assert_eq!(gate.check_application_drained(&proof), Err(GateError::StaleSeal));
    assert_eq!(gate.begin_host_retirement(&proof).err(), Some(GateError::StaleSeal));
    gate.check_host_retirement(&retirement).unwrap();
    drop(proof);
    assert!(matches!(ticket.wait().await, Err(GateError::StaleSeal)));
    assert!(matches!(ticket.wait_application().await, Err(GateError::StaleSeal)));
    assert_eq!(ticket.cancel_before_control().err(), Some(GateError::StaleSeal));
    gate.check_host_retirement(&retirement).unwrap();
    assert!(matches!(gate.admit(OperationKind::Admin), Err(GateError::Sealed)));
    assert!(matches!(gate.seal(), Err(GateError::Sealed)));
    assert!(gate.inner.state.lock().unwrap().legacy_host == LegacyHostState::Retained);
}

#[tokio::test]
async fn dropping_transferred_ticket_preserves_retirement_owner() {
    let (gate, _host) = OperationGate::new_with_legacy_host(1).unwrap();
    let mut ticket = gate.seal().unwrap();
    let retirement = gate.begin_host_retirement(&ticket.wait_application().await.unwrap()).unwrap();
    drop(ticket);
    gate.check_host_retirement(&retirement).unwrap();
    assert!(!gate.stats().unwrap().unconfirmed);
    drop(retirement);
    assert!(gate.stats().unwrap().unconfirmed);
    assert!(gate.stats().unwrap().sealed);
    assert!(matches!(gate.admit(OperationKind::Admin), Err(GateError::Unconfirmed)));
    assert!(matches!(gate.seal(), Err(GateError::Unconfirmed)));
}

#[tokio::test]
async fn wrong_gate_cannot_begin_or_check_retirement() {
    let (gate, _host) = OperationGate::new_with_legacy_host(1).unwrap();
    let (other, _other_host) = OperationGate::new_with_legacy_host(1).unwrap();
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    assert_eq!(other.begin_host_retirement(&proof).err(), Some(GateError::WrongGate));
    gate.check_application_drained(&proof).unwrap();
    let retirement = gate.begin_host_retirement(&proof).unwrap();
    assert_eq!(other.check_host_retirement(&retirement), Err(GateError::WrongGate));
    gate.check_host_retirement(&retirement).unwrap();
}

#[tokio::test]
async fn stale_application_observation_cannot_begin_retirement() {
    let (gate, _host) = OperationGate::new_with_legacy_host(1).unwrap();
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    gate.inner.state.lock().unwrap().phase = Phase::Draining(proof.ticket.id + 1);
    assert_eq!(gate.begin_host_retirement(&proof).err(), Some(GateError::StaleSeal));
}

#[tokio::test]
async fn stale_retirement_token_cannot_confirm_current_seal() {
    let (gate, _host) = OperationGate::new_with_legacy_host(1).unwrap();
    let mut ticket = gate.seal().unwrap();
    let retirement = gate.begin_host_retirement(&ticket.wait_application().await.unwrap()).unwrap();
    gate.inner.state.lock().unwrap().phase = Phase::Retiring(retirement.id + 1);
    assert_eq!(gate.check_host_retirement(&retirement), Err(GateError::StaleSeal));
}

#[tokio::test]
async fn begin_retirement_rechecks_named_application_registry() {
    let (gate, _host) = OperationGate::new_with_legacy_host(2).unwrap();
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    // Public admission is sealed. Inject a late registry entry to test that
    // begin does not trust an earlier observation without atomic revalidation.
    gate.inner.state.lock().unwrap().applications.insert(99, OperationKind::StateWrite);
    assert_eq!(gate.begin_host_retirement(&proof).err(), Some(GateError::NotDrained));
    assert!(matches!(gate.inner.state.lock().unwrap().phase, Phase::Draining(_)));
}

#[tokio::test]
async fn cancelled_result_waiter_does_not_drop_retirement_owned_by_join_task() {
    let (gate, _host) = OperationGate::new_with_legacy_host(1).unwrap();
    let mut ticket = gate.seal().unwrap();
    let retirement = gate.begin_host_retirement(&ticket.wait_application().await.unwrap()).unwrap();
    drop(ticket);
    let (release, released) = tokio::sync::oneshot::channel();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let owned_gate = gate.clone();
    let worker = tokio::spawn(async move {
        released.await.unwrap();
        owned_gate.check_host_retirement(&retirement).unwrap();
        let _ = sender.send(());
        drop(retirement);
    });
    drop(receiver);
    assert!(gate.stats().unwrap().sealed && !gate.stats().unwrap().unconfirmed);
    assert!(matches!(gate.admit(OperationKind::Inference), Err(GateError::Sealed)));
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), worker).await.unwrap().unwrap();
    assert!(gate.stats().unwrap().unconfirmed && gate.stats().unwrap().sealed);
}

#[tokio::test]
async fn lost_host_invalidates_retirement_without_reopening_admission() {
    let (gate, host) = OperationGate::new_with_legacy_host(1).unwrap();
    let mut ticket = gate.seal().unwrap();
    let retirement = gate.begin_host_retirement(&ticket.wait_application().await.unwrap()).unwrap();
    drop(host);
    assert_eq!(gate.check_host_retirement(&retirement), Err(GateError::Unconfirmed));
    assert_eq!(ticket.cancel_before_control().err(), Some(GateError::Unconfirmed));
    assert!(gate.stats().unwrap().sealed);
}

#[tokio::test]
async fn poisoned_retirement_cannot_be_confirmed_or_reopened() {
    let (gate, _host) = OperationGate::new_with_legacy_host(1).unwrap();
    let mut ticket = gate.seal().unwrap();
    let retirement = gate.begin_host_retirement(&ticket.wait_application().await.unwrap()).unwrap();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lock = gate.inner.state.lock().unwrap(); panic!("fixture retirement poison");
    }));
    assert_eq!(gate.check_host_retirement(&retirement), Err(GateError::Poisoned));
    assert_eq!(ticket.cancel_before_control().err(), Some(GateError::Poisoned));
    drop(retirement);
    assert!(gate.inner.state.lock().unwrap_or_else(|e| e.into_inner()).unconfirmed);
}
