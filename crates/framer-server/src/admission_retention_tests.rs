use super::*;
use crate::admission::retain_prepared;
use framer_core::admission::PoolAdmission;

async fn prepared() -> (AppState, Arc<Ports>, PoolAdmission) {
    let (state, ports) = owned();
    let resident = lease_account(&state, id(1)).await.unwrap();
    let operation = state.operation_ownership.as_ref().unwrap().external_cleanup().unwrap();
    (state, ports, PoolAdmission::new(resident, Some(operation)))
}

#[tokio::test]
async fn routing_companion_retains_original_account_without_browser_work() {
    let (state, ports, pool) = prepared().await;
    let mut companion = retain_prepared(&state, &identity(), &pool).unwrap();
    companion.start();
    assert_eq!(slot(&state, id(1)).users, 2);
    pool.complete().unwrap();
    assert_eq!(slot(&state, id(1)).users, 1);
    assert!(management(&state, id(1), false).await.is_err());
    assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1))]);
    companion.complete().unwrap();
    assert_eq!(slot(&state, id(1)).users, 0);
    assert!(!slot(&state, id(1)).uncertain);
}

#[tokio::test]
async fn routing_companion_rejects_another_account_without_mutation() {
    let (state, ports, pool) = prepared().await;
    let mut other = identity();
    other.account = id(2);
    assert_eq!(retain_prepared(&state, &other, &pool).err().unwrap(),
               FailureKind::InvalidScope);
    assert_eq!(slot(&state, id(1)).users, 1);
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 1);
    assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1))]);
}

#[tokio::test]
async fn routing_companion_loss_keeps_native_retention_after_original_completion() {
    let (state, _, pool) = prepared().await;
    let mut companion = retain_prepared(&state, &identity(), &pool).unwrap();
    companion.start();
    pool.complete().unwrap();
    drop(companion);
    assert_eq!(slot(&state, id(1)).users, 1);
    assert!(slot(&state, id(1)).uncertain);
    assert!(management(&state, id(1), false).await.is_err());
}

#[tokio::test]
async fn routing_companion_rejects_sealed_gate_without_status_or_releasing_original() {
    let (state, ports, pool) = prepared().await;
    let seal = state.operation_ownership.as_ref().unwrap().seal().unwrap();
    assert_eq!(retain_prepared(&state, &identity(), &pool).err().unwrap(),
               FailureKind::Ineligible);
    assert_eq!(slot(&state, id(1)).users, 1);
    assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1))]);
    drop(seal);
}

#[tokio::test]
async fn routing_companion_rejects_existing_lifecycle_fence() {
    let (state, ports, pool) = prepared().await;
    let observer = lease_account(&state, id(1)).await.unwrap();
    observer.mark_unconfirmed().unwrap();
    drop(observer);
    assert_eq!(retain_prepared(&state, &identity(), &pool).err().unwrap(),
               FailureKind::Ineligible);
    assert_eq!(slot(&state, id(1)).users, 1);
    assert!(slot(&state, id(1)).uncertain);
    assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1)), ("status", id(1))]);
}
