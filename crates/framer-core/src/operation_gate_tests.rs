use super::*;

#[tokio::test]
async fn all_registered_kinds_delay_drain_until_explicit_completion() {
    for kind in [OperationKind::Inference, OperationKind::Admin,
        OperationKind::SessionWrite, OperationKind::StateWrite,
        OperationKind::Background, OperationKind::Authorization,
        OperationKind::ExternalCleanup] {
        let gate = OperationGate::new(2).unwrap();
        let owner = gate.admit(kind).unwrap();
        let mut ticket = gate.seal().unwrap();
        assert!(matches!(gate.admit(kind), Err(GateError::Sealed)));
        let mut draining = std::pin::pin!(ticket.wait());
        assert!(futures::poll!(draining.as_mut()).is_pending());
        assert_eq!(gate.stats().unwrap().active, 1);
        owner.complete().unwrap();
        let lease = draining.await.unwrap();
        gate.check_held(&lease).unwrap();
    }
}

#[tokio::test]
async fn registered_cleanup_survives_parent_completion_during_seal() {
    let gate = OperationGate::new(2).unwrap();
    let parent = gate.admit(OperationKind::Inference).unwrap();
    let mut ticket = gate.seal().unwrap();
    let cleanup = parent.register_cleanup().unwrap();
    parent.complete().unwrap();
    let mut draining = std::pin::pin!(ticket.wait());
    assert!(futures::poll!(draining.as_mut()).is_pending());
    cleanup.complete().unwrap();
    let lease = draining.await.unwrap();
    gate.check_held(&lease).unwrap();
}

#[tokio::test]
async fn seal_race_admits_whole_owner_or_rejects_without_registration() {
    for _ in 0..16 {
        let gate = OperationGate::new(2).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let a = gate.clone(); let start_a = barrier.clone();
        let request = std::thread::spawn(move || {
            start_a.wait(); a.admit(OperationKind::Admin)
        });
        let b = gate.clone(); let start_b = barrier.clone();
        let sealing = std::thread::spawn(move || { start_b.wait(); b.seal().unwrap() });
        barrier.wait();
        let owner = request.join().unwrap();
        let mut ticket = sealing.join().unwrap();
        match owner {
            Ok(owner) => { assert_eq!(gate.stats().unwrap().active, 1); owner.complete().unwrap(); }
            Err(error) => { assert_eq!(error, GateError::Sealed); assert_eq!(gate.stats().unwrap().active, 0); }
        }
        let lease = ticket.wait().await.unwrap();
        gate.check_held(&lease).unwrap();
    }
}

#[tokio::test]
async fn drain_timeout_can_cancel_only_pre_control_and_preserves_active_work() {
    let gate = OperationGate::new(2).unwrap();
    let owner = gate.admit(OperationKind::Inference).unwrap();
    let mut ticket = gate.seal().unwrap();
    assert!(tokio::time::timeout(std::time::Duration::from_millis(1),
        ticket.wait()).await.is_err());
    ticket.cancel_before_control().unwrap();
    assert_eq!(gate.stats().unwrap(), GateStats { active: 1, sealed: false, unconfirmed: false });
    let next = gate.admit(OperationKind::Admin).unwrap();
    next.complete().unwrap();
    owner.complete().unwrap();
}

#[tokio::test]
async fn dropped_operation_is_unconfirmed_even_when_active_count_zero() {
    let gate = OperationGate::new(1).unwrap();
    let owner = gate.admit(OperationKind::Background).unwrap();
    let mut ticket = gate.seal().unwrap();
    drop(owner);
    assert_eq!(gate.stats().unwrap().active, 0);
    assert!(matches!(ticket.wait().await, Err(GateError::Unconfirmed)));
    assert_eq!(ticket.cancel_before_control().err(), Some(GateError::Unconfirmed));
    assert!(matches!(gate.admit(OperationKind::Inference), Err(GateError::Unconfirmed)));
}

#[tokio::test]
async fn dropping_drain_lease_keeps_sticky_seal_and_denies_second_owner() {
    let gate = OperationGate::new(1).unwrap();
    let mut ticket = gate.seal().unwrap();
    let lease = ticket.wait().await.unwrap();
    assert!(matches!(gate.seal(), Err(GateError::Sealed)));
    drop(lease);
    assert_eq!(gate.stats().unwrap(), GateStats { active: 0, sealed: true, unconfirmed: true });
    assert!(matches!(gate.seal(), Err(GateError::Unconfirmed)));
}

#[test]
fn bounded_registration_and_abandoned_ticket_fail_closed() {
    let gate = OperationGate::new(1).unwrap();
    let owner = gate.admit(OperationKind::Inference).unwrap();
    assert!(matches!(owner.register_cleanup(), Err(GateError::Full)));
    assert!(matches!(gate.admit(OperationKind::SessionWrite), Err(GateError::Full)));
    let ticket = gate.seal().unwrap();
    drop(ticket);
    owner.complete().unwrap();
    assert!(matches!(gate.seal(), Err(GateError::Unconfirmed)));
}

#[tokio::test]
async fn wrong_gate_lease_and_poisoned_gate_cannot_authorize_guard() {
    let first = OperationGate::new(1).unwrap();
    let other = OperationGate::new(1).unwrap();
    let control = ControlGuard::bind(first.clone(), Arc::new(crate::wave::WaveGuard::new()));
    let mut ticket = other.seal().unwrap();
    let wrong = ticket.wait().await.unwrap();
    assert_eq!(control.check(&wrong), Err(GuardCheckError::Gate(GateError::WrongGate)));
    let mut ticket = first.seal().unwrap();
    let lease = ticket.wait().await.unwrap();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lock = first.inner.state.lock().unwrap(); panic!("fixture gate poison");
    }));
    assert_eq!(control.check(&lease), Err(GuardCheckError::Gate(GateError::Poisoned)));
}

#[tokio::test]
async fn completed_wait_cannot_cancel_an_issued_lease() {
    let gate = OperationGate::new(1).unwrap();
    let mut ticket = gate.seal().unwrap();
    let lease = ticket.wait().await.unwrap();
    assert_eq!(ticket.cancel_before_control().err(), Some(GateError::StaleSeal));
    gate.check_held(&lease).unwrap();
    assert!(gate.stats().unwrap().sealed);
}

#[tokio::test]
async fn dropped_result_waiter_does_not_complete_supervisor_owned_operation() {
    let gate = OperationGate::new(1).unwrap();
    let owner = gate.admit(OperationKind::Inference).unwrap();
    let (release, released) = tokio::sync::oneshot::channel();
    let (sender, result) = tokio::sync::oneshot::channel::<()>();
    let worker = tokio::spawn(async move {
        let _ = released.await;
        owner.complete().unwrap();
        let _ = sender.send(());
    });
    drop(result);
    let mut ticket = gate.seal().unwrap();
    let mut draining = std::pin::pin!(ticket.wait());
    assert!(futures::poll!(draining.as_mut()).is_pending());
    assert_eq!(gate.stats().unwrap().active, 1);
    release.send(()).unwrap();
    worker.await.unwrap();
    let lease = draining.await.unwrap();
    gate.check_held(&lease).unwrap();
}
