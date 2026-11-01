use super::*;
use serde_json::json;

#[tokio::test]
async fn incomplete_startup_cannot_be_retried_or_reported_retired() {
    let (gate, _host) = OperationGate::new_with_legacy_host(8).unwrap();
    let (bridge, control) = SidecarBridge::new_retirable("unused-host-fixture", gate.clone());
    let boundary = bridge.retirement.as_ref().unwrap();
    boundary.begin_start().unwrap();
    assert!(matches!(boundary.begin_start(), Err(RetirementError::ProcessUnconfirmed)));
    assert!(boundary.check_ready().is_err());
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    assert!(matches!(control.seal_admission(&proof).await,
        Err(RetirementError::ProcessUnconfirmed)));
    assert!(gate.check_application_drained(&proof).is_ok());
    boundary.finish_start().unwrap();
    assert!(boundary.check_ready().is_ok());
    assert!(boundary.begin_start().is_err());
    assert!(control.seal_admission(&proof).await.is_ok());
}

#[tokio::test]
async fn admission_waiting_for_lifecycle_lock_rechecks_local_calls() {
    let (gate, _host) = OperationGate::new_with_legacy_host(8).unwrap();
    let (bridge, control) = SidecarBridge::new_retirable("unused-host-fixture", gate.clone());
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    let lifecycle = bridge.inner.lock().await;
    let mut closing = Box::pin(control.seal_admission(&proof));
    assert!(futures::poll!(closing.as_mut()).is_pending());
    let local = bridge.retirement.as_ref().unwrap().enter().unwrap();
    drop(lifecycle);
    assert!(matches!(closing.await, Err(RetirementError::Busy)));
    drop(local);
    assert!(control.seal_admission(&proof).await.is_ok());
}

#[test]
fn poisoned_accounting_rejects_admission_and_drop_does_not_panic() {
    let (gate, _host) = OperationGate::new_with_legacy_host(8).unwrap();
    let boundary = Arc::new(AdmissionBoundary::new(gate));
    let call = boundary.enter().unwrap();
    let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _state = boundary.state.lock().unwrap();
        panic!("fixture poison");
    }));
    assert!(poisoned.is_err());
    assert!(matches!(boundary.enter(), Err(RetirementError::Poisoned)));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(call))).is_ok());
    let state = match boundary.state.lock() {
        Err(error) => error.into_inner(),
        Ok(_) => panic!("expected poisoned fixture accounting"),
    };
    assert!(state.uncertain);
    assert_eq!(state.active, 0);
}

#[tokio::test]
async fn existing_local_call_delays_admission_closure() {
    let (gate, _host) = OperationGate::new_with_legacy_host(8).unwrap();
    let (bridge, control) = SidecarBridge::new_retirable("unused-host-fixture", gate.clone());
    let boundary = bridge.retirement.as_ref().unwrap();
    let call = boundary.enter().unwrap();
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    assert!(matches!(control.seal_admission(&proof).await, Err(RetirementError::Busy)));
    assert!(boundary.check_open().is_ok());
    assert_eq!(boundary.state.lock().unwrap().active, 1);
    drop(call);
    assert!(control.seal_admission(&proof).await.is_ok());
}

#[tokio::test]
async fn dropping_closed_token_does_not_reopen_or_complete_global_drain() {
    let (gate, _host) = OperationGate::new_with_legacy_host(8).unwrap();
    let (bridge, control) = SidecarBridge::new_retirable("unused-host-fixture", gate.clone());
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    drop(control.seal_admission(&proof).await.unwrap());
    assert!(matches!(control.seal_admission(&proof).await, Err(RetirementError::Closed)));
    assert!(bridge.call("fixture", json!({})).await.is_err());
    drop(proof);
    assert!(matches!(ticket.wait().await, Err(GateError::Unconfirmed)));
}

#[tokio::test]
async fn transferred_seal_cannot_reopen_while_host_is_retiring() {
    let (gate, _host) = OperationGate::new_with_legacy_host(8).unwrap();
    let (bridge, control) = SidecarBridge::new_retirable("unused-host-fixture", gate.clone());
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    let retiring = control.seal_admission(&proof).await.unwrap();
    drop(proof);
    assert!(matches!(ticket.cancel_before_control(), Err(GateError::StaleSeal)));
    let joined = retiring.wait_local_tasks().await.unwrap();
    assert_eq!(joined.task_count(), 0);
    assert!(joined.leader_status().is_none());
    assert!(joined.incarnation().is_none());
    assert!(bridge.call("fixture", json!({})).await.is_err());
    assert!(gate.check_host_retirement(&retiring.authority).is_ok());
}

#[tokio::test]
async fn closed_new_bridge_cannot_start_a_process() {
    let (gate, _host) = OperationGate::new_with_legacy_host(8).unwrap();
    let (bridge, control) = SidecarBridge::new_retirable("unused-host-fixture", gate.clone());
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    let closed = control.seal_admission(&proof).await.unwrap();
    assert!(closed.matches_bridge(&bridge));
    assert_eq!(closed.observed_incarnation(), None);
    assert!(bridge.call("fixture", json!({})).await.unwrap_err().contains("closed for shutdown"));
    assert!(bridge.sidecar().await.unwrap_err().contains("closed for shutdown"));
    assert!(bridge.inner.lock().await.is_none());
}

#[tokio::test]
async fn another_gate_cannot_close_this_bridge() {
    let (gate, _host) = OperationGate::new_with_legacy_host(8).unwrap();
    let (other, _other_host) = OperationGate::new_with_legacy_host(8).unwrap();
    let (bridge, control) = SidecarBridge::new_retirable("unused-host-fixture", gate);
    let mut ticket = other.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    assert!(matches!(control.seal_admission(&proof).await,
        Err(RetirementError::Gate(GateError::WrongGate))));
    assert!(bridge.retirement.as_ref().unwrap().check_open().is_ok());
    assert!(bridge.inner.lock().await.is_none());
}
