use super::*;
use crate::{operation_gate::{OperationGate, OperationKind},
    resident_pool::{Acquire, PoolError, ResidentPool, ResidentStatus}};

fn fixture() -> (ResidentPool, OperationGate, PoolAdmission) {
    let pool = ResidentPool::new(2).unwrap();
    let gate = OperationGate::new(8).unwrap();
    let Acquire::Lease(lease) = pool.try_acquire(AccountId::new(), None).unwrap()
        else { panic!("fresh slot"); };
    let owner = gate.admit(OperationKind::Inference).unwrap();
    (pool, gate, PoolAdmission::new(lease, Some(owner)))
}

#[test]
fn unused_preparation_releases_both_owners_without_claiming_browser_absence() {
    let (pool, gate, admission) = fixture();
    drop(admission);
    let snapshot = pool.snapshot().unwrap();
    assert_eq!(snapshot.occupied, 1);
    assert_eq!(snapshot.residents[0].users, 0);
    assert_eq!(snapshot.residents[0].status, ResidentStatus::Idle);
    assert_eq!(gate.stats().unwrap().active, 0);
    assert!(!gate.stats().unwrap().unconfirmed);
}

#[test]
fn abandoned_started_work_cannot_be_parked_or_replaced() {
    let (pool, gate, mut admission) = fixture();
    let account = admission.account();
    admission.start();
    drop(admission);
    let snapshot = pool.snapshot().unwrap();
    assert_eq!(snapshot.residents[0].users, 1);
    assert_eq!(snapshot.residents[0].status, ResidentStatus::Uncertain);
    assert!(matches!(pool.begin_park(account), Err(PoolError::AccountBusy)));
    assert!(matches!(pool.try_acquire(account, None), Err(PoolError::AccountBusy)));
    assert!(gate.stats().unwrap().unconfirmed);
    assert!(matches!(pool.try_acquire(AccountId::new(), None), Ok(Acquire::Lease(_))));
}

#[test]
fn affirmative_cleanup_releases_work_but_retains_the_resident_slot() {
    let (pool, gate, mut admission) = fixture();
    admission.start();
    admission.complete().unwrap();
    let snapshot = pool.snapshot().unwrap();
    assert_eq!(snapshot.occupied, 1);
    assert_eq!(snapshot.residents[0].users, 0);
    assert_eq!(snapshot.residents[0].status, ResidentStatus::Idle);
    assert_eq!(gate.stats().unwrap().active, 0);
    assert!(!gate.stats().unwrap().unconfirmed);
}
