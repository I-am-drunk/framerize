use super::*;

#[test]
fn fenced_slot_keeps_its_failure_until_confirmed_close() {
    let pool = ResidentPool::new(1).unwrap();
    let owner = lease(&pool, 1);
    owner.mark_unconfirmed_with_cause("browser status did not reply within 5 seconds").unwrap();
    owner.mark_unconfirmed_with_cause("later less specific error").unwrap();
    drop(owner);
    let state = pool.snapshot().unwrap();
    assert_eq!(state.residents[0].cleanup_cause.as_deref(),
        Some("browser status did not reply within 5 seconds"));
    assert!(pool.try_acquire(id(1), None).is_err());
    let mut close = pool.begin_park(id(1)).unwrap();
    close.close_started().unwrap();
    close.confirm_close().unwrap();
    let _next = lease(&pool, 1);
    assert!(pool.snapshot().unwrap().residents[0].cleanup_cause.is_none());
}

#[test]
fn only_matching_busy_receipts_cancel_close_without_freeing_capacity() {
    use crate::browser_lifecycle::{BrowserAccountState as State, IdleCloseOutcome};
    let expected = BrowserAccountStatus { account_id: id(1), host_generation: Some("host".into()),
        account_incarnation: 1, state: State::Resident, active_commands: 0, cleanup_cause: None };
    for case in 0..9 {
        let pool = ResidentPool::new(1).unwrap();
        drop(lease(&pool, 1));
        let mut ticket = pool.begin_park(id(1)).unwrap();
        ticket.close_started().unwrap();
        let mut result = IdleCloseResult { outcome: IdleCloseOutcome::Busy,
            status: BrowserAccountStatus { active_commands: 1, ..expected.clone() } };
        match case {
            1 => result.status.account_id = id(2),
            2 => result.status.host_generation = Some("other".into()),
            3 => result.status.account_incarnation = 2,
            4 => result.outcome = IdleCloseOutcome::StaleGeneration,
            5 => result.status.state = State::Absent,
            6 => result.status.state = State::Unconfirmed,
            7 => result.status.active_commands = 0,
            8 => { result.status.state = State::Starting; result.status.active_commands = 0; },
            _ => {},
        }
        let valid = case == 0 || case == 8;
        assert_eq!(ticket.confirm_busy(&expected, &result).is_ok(), valid, "case {case}");
        let snapshot = pool.snapshot().unwrap();
        assert_eq!(snapshot.occupied, 1);
        assert_eq!(snapshot.residents[0].account, id(1));
        assert_eq!(snapshot.residents[0].uncertain, !valid);
        assert_eq!(snapshot.residents[0].replacement, None);
    }
}

#[test]
fn existing_only_never_allocates_or_selects_a_victim() {
    let pool = ResidentPool::new(1).unwrap();
    assert!(matches!(pool.try_acquire_idle_existing(id(1)), Err(PoolError::NotResident)));
    assert_eq!(pool.snapshot().unwrap().occupied, 0);
    drop(lease(&pool, 1));
    assert!(matches!(pool.try_acquire_idle_existing(id(2)), Err(PoolError::NotResident)));
    assert_eq!(status(&pool, 1), ResidentStatus::Idle);
    let held = pool.try_acquire_idle_existing(id(1)).unwrap();
    assert_eq!(pool.snapshot().unwrap().residents[0].users, 1);
    assert!(matches!(pool.try_acquire_idle_existing(id(1)), Err(PoolError::AccountBusy)));
    assert!(matches!(pool.begin_park(id(1)), Err(PoolError::AccountBusy)));
    drop(held);
    assert_eq!(status(&pool, 1), ResidentStatus::Idle);
}

#[test]
fn existing_only_refuses_closing_and_uncertain_slots() {
    let pool = ResidentPool::new(1).unwrap();
    drop(lease(&pool, 1));
    let close = pool.begin_park(id(1)).unwrap();
    assert!(matches!(pool.try_acquire_idle_existing(id(1)), Err(PoolError::AccountBusy)));
    close.cancel_before_io().unwrap();
    pool.try_acquire_idle_existing(id(1)).unwrap().retain_unconfirmed();
    assert!(matches!(pool.try_acquire_idle_existing(id(1)), Err(PoolError::AccountBusy)));
    assert_eq!(pool.snapshot().unwrap().residents[0].users, 1);
}

#[test]
fn ordinary_drop_preserves_a_separately_retained_native_user() {
    for mark_first in [true, false] {
        let pool = ResidentPool::new(1).unwrap();
        let native = lease(&pool, 1);
        let ordinary = lease(&pool, 1);
        ordinary.ensure_admissible().unwrap();
        if mark_first { ordinary.mark_unconfirmed().unwrap(); }
        native.retain_unconfirmed();
        ordinary.mark_unconfirmed().unwrap();
        assert!(matches!(ordinary.ensure_admissible(), Err(PoolError::AccountBusy)));
        assert_eq!(pool.snapshot().unwrap().residents[0].users, 2);
        drop(ordinary);
        let snapshot = pool.snapshot().unwrap();
        assert_eq!(snapshot.occupied, 1);
        assert_eq!(snapshot.residents[0].users, 1);
        assert_eq!(snapshot.residents[0].status, ResidentStatus::Uncertain);
        assert!(pool.idle_ids().unwrap().is_empty());
        assert!(matches!(pool.try_acquire(id(1), None), Err(PoolError::AccountBusy)));
        assert!(matches!(pool.try_acquire(id(2), None), Err(PoolError::NoIdleSlot)));
        assert!(matches!(pool.begin_park(id(1)), Err(PoolError::AccountBusy)));
    }
}

#[test]
fn negative_browser_observation_retains_users_and_capacity() {
    let pool = ResidentPool::new(1).unwrap();
    let first = lease(&pool, 1);
    let second = lease(&pool, 1);
    first.mark_unconfirmed().unwrap();
    let snapshot = pool.snapshot().unwrap();
    assert_eq!(snapshot.occupied, 1);
    assert_eq!(snapshot.residents[0].users, 2);
    assert_eq!(snapshot.residents[0].status, ResidentStatus::Uncertain);
    assert!(matches!(pool.try_acquire(id(1), None), Err(PoolError::AccountBusy)));
    assert!(matches!(pool.try_acquire(id(2), None), Err(PoolError::NoIdleSlot)));
    assert!(matches!(pool.begin_park(id(1)), Err(PoolError::AccountBusy)));
    drop(first);
    drop(second);
    assert!(pool.idle_ids().unwrap().is_empty());
    assert_eq!(status(&pool, 1), ResidentStatus::Uncertain);
    assert_eq!(pool.snapshot().unwrap().occupied, 1);
}

fn id(n: u128) -> AccountId { AccountId(uuid::Uuid::from_u128(n)) }

#[test]
fn negative_browser_observation_needs_a_confirmed_park_to_release() {
    let pool = ResidentPool::new(1).unwrap();
    let owner = lease(&pool, 1);
    owner.mark_unconfirmed().unwrap();
    owner.mark_unconfirmed().unwrap();
    drop(owner);
    pool.begin_park(id(1)).unwrap().cancel_before_io().unwrap();
    assert_eq!(status(&pool, 1), ResidentStatus::Uncertain);
    let mut close = pool.begin_park(id(1)).unwrap();
    close.close_started().unwrap();
    close.confirm_close().unwrap();
    assert_eq!(pool.snapshot().unwrap().occupied, 0);
    assert_eq!(lease(&pool, 2).account(), id(2));
}
fn lease(pool: &ResidentPool, n: u128) -> ResidentLease {
    match pool.try_acquire(id(n), None).unwrap() {
        Acquire::Lease(lease) => lease,
        Acquire::Evict(_) => panic!("unexpected eviction"),
    }
}
fn evict(pool: &ResidentPool, n: u128, preferred: Option<AccountId>) -> EvictionTicket {
    match pool.try_acquire(id(n), preferred).unwrap() {
        Acquire::Evict(ticket) => ticket,
        Acquire::Lease(_) => panic!("expected eviction reservation"),
    }
}
fn status(pool: &ResidentPool, n: u128) -> ResidentStatus {
    pool.snapshot().unwrap().residents.into_iter()
        .find(|s| s.account == id(n)).unwrap().status
}

#[test]
fn six_slots_and_shared_claims_hold_until_last_chain_drops() {
    let pool = ResidentPool::new(6).unwrap();
    let mut leases: Vec<_> = (1..=6).map(|n| lease(&pool, n)).collect();
    let shared = lease(&pool, 1);
    assert_eq!(pool.snapshot().unwrap().occupied, 6);
    assert!(matches!(pool.try_acquire(id(7), None), Err(PoolError::NoIdleSlot)));
    drop(leases.remove(0));
    assert_eq!(status(&pool, 1), ResidentStatus::Active);
    assert!(matches!(pool.begin_park(id(1)), Err(PoolError::AccountBusy)));
    drop(shared);
    assert_eq!(pool.idle_ids().unwrap(), vec![id(1)]);
    let ticket = evict(&pool, 7, None);
    assert_eq!(ticket.victim(), id(1));
    assert_eq!(pool.snapshot().unwrap().occupied, 6);
    ticket.cancel_before_io().unwrap();
}

#[test]
fn eviction_reserves_victim_and_target_until_confirmed_close() {
    let pool = ResidentPool::new(1).unwrap();
    drop(lease(&pool, 1));
    let mut ticket = evict(&pool, 2, None);
    assert_eq!(ticket.target(), id(2));
    assert!(matches!(pool.try_acquire(id(1), None), Err(PoolError::AccountBusy)));
    assert!(matches!(pool.try_acquire(id(2), None), Err(PoolError::AccountBusy)));
    assert!(matches!(pool.try_acquire(id(3), None), Err(PoolError::NoIdleSlot)));
    assert!(matches!(pool.begin_park(id(1)), Err(PoolError::AccountBusy)));
    ticket.close_started().unwrap();
    assert!(pool.snapshot().unwrap().residents[0].close_started);
    let replacement = ticket.confirm_close().unwrap();
    assert_eq!(replacement.account(), id(2));
    assert_eq!(pool.resident_ids().unwrap(), vec![id(2)]);
    assert_eq!(status(&pool, 2), ResidentStatus::Active);
}

#[test]
fn dropping_unstarted_or_started_close_keeps_uncertain_slot() {
    for start in [false, true] {
        let pool = ResidentPool::new(1).unwrap();
        drop(lease(&pool, 1));
        let mut ticket = evict(&pool, 2, None);
        if start { ticket.close_started().unwrap(); }
        drop(ticket);
        assert_eq!(status(&pool, 1), ResidentStatus::Uncertain);
        assert!(pool.idle_ids().unwrap().is_empty());
        assert_eq!(pool.snapshot().unwrap().occupied, 1);
        assert!(matches!(pool.try_acquire(id(2), None), Err(PoolError::NoIdleSlot)));
        assert!(matches!(pool.try_acquire(id(1), None), Err(PoolError::AccountBusy)));
    }
}

#[test]
fn only_explicit_pre_io_cancel_restores_idle() {
    let pool = ResidentPool::new(1).unwrap();
    drop(lease(&pool, 1));
    evict(&pool, 2, None).cancel_before_io().unwrap();
    assert_eq!(status(&pool, 1), ResidentStatus::Idle);
    let mut ticket = evict(&pool, 2, None);
    ticket.close_started().unwrap();
    assert!(matches!(ticket.close_started(), Err(PoolError::CloseState)));
    assert!(matches!(ticket.cancel_before_io(), Err(PoolError::CloseState)));
    assert_eq!(status(&pool, 1), ResidentStatus::Uncertain);
    pool.begin_park(id(1)).unwrap().cancel_before_io().unwrap();
    assert_eq!(status(&pool, 1), ResidentStatus::Uncertain);
}

#[test]
fn same_victim_retry_reconciles_uncertainty_and_frees_capacity() {
    let pool = ResidentPool::new(1).unwrap();
    drop(lease(&pool, 1));
    drop(evict(&pool, 2, None));
    assert!(matches!(pool.begin_park(id(2)), Err(PoolError::NotResident)));
    let mut park = pool.begin_park(id(1)).unwrap();
    assert_eq!(park.victim(), id(1));
    assert!(pool.snapshot().unwrap().residents[0].uncertain);
    assert!(matches!(pool.begin_park(id(1)), Err(PoolError::AccountBusy)));
    park.close_started().unwrap();
    park.confirm_close().unwrap();
    assert_eq!(pool.snapshot().unwrap().occupied, 0);
    let next = lease(&pool, 2);
    assert_eq!(next.account(), id(2));
}

#[test]
fn confirmation_without_started_close_fails_closed_for_both_ticket_types() {
    let pool = ResidentPool::new(1).unwrap();
    drop(lease(&pool, 1));
    assert!(matches!(evict(&pool, 2, None).confirm_close(), Err(PoolError::CloseState)));
    assert_eq!(status(&pool, 1), ResidentStatus::Uncertain);
    assert!(matches!(pool.begin_park(id(1)).unwrap().confirm_close(), Err(PoolError::CloseState)));
    assert_eq!(status(&pool, 1), ResidentStatus::Uncertain);
}

#[test]
fn parked_idle_cancellation_restores_idle_but_started_drop_is_uncertain() {
    let pool = ResidentPool::new(1).unwrap();
    drop(lease(&pool, 1));
    pool.begin_park(id(1)).unwrap().cancel_before_io().unwrap();
    assert_eq!(status(&pool, 1), ResidentStatus::Idle);
    let mut park = pool.begin_park(id(1)).unwrap();
    park.close_started().unwrap();
    drop(park);
    assert_eq!(status(&pool, 1), ResidentStatus::Uncertain);
    assert!(pool.snapshot().unwrap().residents[0].uncertain);
}

#[test]
fn idle_lru_is_deterministic_and_valid_preference_wins() {
    let pool = ResidentPool::new(2).unwrap();
    let first = lease(&pool, 1);
    let second = lease(&pool, 2);
    drop(second);
    drop(first);
    assert_eq!(pool.idle_ids().unwrap(), vec![id(2), id(1)]);
    let preferred = evict(&pool, 3, Some(id(1)));
    assert_eq!(preferred.victim(), id(1));
    preferred.cancel_before_io().unwrap();
    let fallback = evict(&pool, 3, Some(id(99)));
    assert_eq!(fallback.victim(), id(2));
    fallback.cancel_before_io().unwrap();
    for (preferred, protected, expected) in [
        (Some(id(1)), id(1), id(2)),
        (None, id(2), id(1)),
    ] {
        let ticket = match pool.try_acquire_excluding(id(3), preferred, &[protected]).unwrap() {
            Acquire::Evict(ticket) => ticket,
            Acquire::Lease(_) => panic!("full pool admitted a replacement without closing an idle victim"),
        };
        assert_eq!(ticket.victim(), expected);
        ticket.cancel_before_io().unwrap();
    }
    assert!(matches!(pool.try_acquire_excluding(id(3), Some(id(2)), &[id(1), id(2)]),
        Err(PoolError::NoIdleSlot)));
    assert_eq!(pool.idle_ids().unwrap(), vec![id(2), id(1)]);
    let unchanged = pool.snapshot().unwrap();
    assert_eq!(unchanged.occupied, 2);
    assert_eq!(unchanged.capacity, 2);
    assert!(unchanged.residents.iter().all(|slot| slot.users == 0
        && slot.status == ResidentStatus::Idle && slot.replacement.is_none()
        && !slot.close_started && !slot.uncertain));
}

#[test]
fn shrink_requires_confirmed_parking_and_growth_preserves_busy_leases() {
    assert!(matches!(ResidentPool::new(0), Err(PoolError::InvalidCapacity)));
    let pool = ResidentPool::new(2).unwrap();
    let active = lease(&pool, 1);
    drop(lease(&pool, 2));
    assert!(matches!(pool.set_capacity(0), Err(PoolError::InvalidCapacity)));
    assert!(matches!(pool.set_capacity(1), Err(PoolError::BusyShrink)));
    assert!(matches!(pool.begin_park(id(1)), Err(PoolError::AccountBusy)));
    let mut park = pool.begin_park(id(2)).unwrap();
    assert!(matches!(pool.set_capacity(1), Err(PoolError::BusyShrink)));
    park.close_started().unwrap();
    park.confirm_close().unwrap();
    pool.set_capacity(1).unwrap();
    assert_eq!(pool.capacity().unwrap(), 1);
    pool.set_capacity(3).unwrap();
    assert_eq!(status(&pool, 1), ResidentStatus::Active);
    assert_eq!(active.account(), id(1));
    let _second = lease(&pool, 2);
    let _third = lease(&pool, 3);
    assert_eq!(pool.snapshot().unwrap().occupied, 3);
}

#[test]
fn concurrent_same_account_leases_share_one_slot() {
    let pool = ResidentPool::new(1).unwrap();
    let claimed = Arc::new(std::sync::Barrier::new(7));
    let release = Arc::new(std::sync::Barrier::new(7));
    let threads: Vec<_> = (0..6).map(|_| {
        let (pool, claimed, release) = (pool.clone(), claimed.clone(), release.clone());
        std::thread::spawn(move || {
            let _lease = lease(&pool, 1);
            claimed.wait();
            release.wait();
        })
    }).collect();
    claimed.wait();
    let snapshot = pool.snapshot().unwrap();
    assert_eq!(snapshot.occupied, 1);
    assert_eq!(snapshot.residents[0].users, 6);
    assert!(matches!(pool.try_acquire(id(2), None), Err(PoolError::NoIdleSlot)));
    release.wait();
    for thread in threads { thread.join().unwrap(); }
    assert_eq!(pool.idle_ids().unwrap(), vec![id(1)]);
}

#[test]
fn growth_cannot_duplicate_an_account_reserved_by_inflight_eviction() {
    let pool = ResidentPool::new(1).unwrap();
    drop(lease(&pool, 1));
    let mut ticket = evict(&pool, 2, None);
    pool.set_capacity(2).unwrap();
    assert!(matches!(pool.try_acquire(id(2), None), Err(PoolError::AccountBusy)));
    let other = lease(&pool, 3);
    ticket.close_started().unwrap();
    let replacement = ticket.confirm_close().unwrap();
    assert_eq!(pool.snapshot().unwrap().occupied, 2);
    assert_eq!(replacement.account(), id(2));
    assert_eq!(other.account(), id(3));
}
