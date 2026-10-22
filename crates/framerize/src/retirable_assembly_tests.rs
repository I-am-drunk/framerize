use super::*;
use framer_core::{ids::AccountId, operation_gate::OperationKind};
use std::{future::{poll_fn, Future}, task::Poll};

fn new_host() -> (RetirableServingHost, PathBuf) {
    let path = std::env::temp_dir().join(format!("retirable-assembly-{}", uuid::Uuid::new_v4()));
    assert!(!path.exists());
    let host = RetirableServingHost::new(&path, Arc::new(WaveGuard::new()), 16).unwrap();
    (host, path)
}

#[test]
fn invalid_gate_capacity_does_not_invoke_host_factory() {
    let called = std::cell::Cell::new(false);
    let result = ServingOwnership::new_with_host_factory(
        Arc::new(WaveGuard::new()), 0, |_| called.set(true));
    assert!(matches!(result, Err(GateError::InvalidLimit)));
    assert!(!called.get());
}

#[tokio::test]
async fn own_application_proof_seals_the_constructed_host_and_its_clones() {
    let (host, path) = new_host();
    let browser = host.browser();
    let mut ticket = host.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    let sealed = host.begin_retirement(&proof).await.unwrap();
    assert!(sealed.matches_bridge(&host.browser));
    assert!(sealed.observed_incarnation().is_none());
    assert!(browser.castle_token(&AccountId::new()).await.is_err());
    assert!(!path.exists());
}

#[tokio::test]
async fn unrelated_application_proof_cannot_retire_the_assembled_host() {
    let (first, first_path) = new_host();
    let (second, second_path) = new_host();
    let mut first_ticket = first.seal().unwrap();
    let first_proof = first_ticket.wait_application().await.unwrap();
    assert!(matches!(second.begin_retirement(&first_proof).await,
        Err(RetirementError::Gate(GateError::WrongGate))));
    let mut second_ticket = second.seal().unwrap();
    let second_proof = second_ticket.wait_application().await.unwrap();
    let sealed = second.begin_retirement(&second_proof).await.unwrap();
    assert!(sealed.matches_bridge(&second.browser));
    assert!(!sealed.matches_bridge(&first.browser));
    assert!(!first_path.exists() && !second_path.exists());
}

#[tokio::test]
async fn local_joins_cannot_reopen_or_complete_the_global_host_barrier() {
    let (host, path) = new_host();
    let mut ticket = host.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    let sealed = host.begin_retirement(&proof).await.unwrap();
    let joined = sealed.wait_local_tasks().await.unwrap();
    assert_eq!(joined.task_count(), 0);
    assert!(joined.leader_status().is_none());
    drop(proof);
    assert!(matches!(ticket.wait().await, Err(GateError::StaleSeal)));
    assert!(host.ownership().unmanaged_host_close_required());
    assert!(!path.exists());
}

#[tokio::test]
async fn admitted_application_work_delays_retirement_without_being_cancelled() {
    let (host, path) = new_host();
    let (release, wait_release) = tokio::sync::oneshot::channel();
    let owner = host.ownership();
    let work = framer_server::operation_ownership::spawn(
        Some(&owner), OperationKind::Inference,
        async move { wait_release.await.unwrap() },
    ).unwrap();
    let mut ticket = host.seal().unwrap();
    let mut waiting = Box::pin(ticket.wait_application());
    poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    }).await;
    drop(waiting);
    assert!(!work.is_finished());
    assert!(!path.exists());
    release.send(()).unwrap();
    work.await.unwrap();
    let proof = ticket.wait_application().await.unwrap();
    let sealed = host.begin_retirement(&proof).await.unwrap();
    assert!(sealed.matches_bridge(&host.browser));
    assert!(!path.exists());
}
