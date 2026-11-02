use super::*;
use framer_core::resident_pool::{Acquire, ResidentLease};

fn hold(p: &ServingAccountPolicy, n: u128) -> ResidentLease {
    match p.residents.try_acquire(account(n).id, None).unwrap() {
        Acquire::Lease(lease) => lease,
        Acquire::Evict(_) => panic!("unexpected replacement"),
    }
}

#[test]
fn idle_warm_account_wins_an_equal_load_tie() {
    let p = ServingAccountPolicy::default();
    drop(hold(&p, 2));
    for _ in 0..8 {
        assert_eq!(p.select_at(vec![account(1), account(2)], None, 1000).unwrap().id,
            account(2).id);
    }
}

#[test]
fn normal_requests_eventually_use_all_twelve_accounts_with_six_residents() {
    let p = ServingAccountPolicy::default();
    let pool: Vec<_> = (1..=12).map(account).collect();
    for n in 1..=6 { drop(hold(&p, n)); }
    let mut selected = std::collections::HashSet::new();
    for _ in 0..120 {
        let chosen = p.select_at(pool.clone(), None, 1000).unwrap();
        selected.insert(chosen.id);
        let lease = match p.residents.try_acquire(chosen.id, None).unwrap() {
            Acquire::Lease(lease) => lease,
            Acquire::Evict(mut ticket) => {
                // This fixture models a successful close, without browser I/O.
                ticket.close_started().unwrap();
                ticket.confirm_close().unwrap()
            }
        };
        assert_eq!(p.residents.snapshot().unwrap().occupied, 6);
        drop(lease);
    }
    assert_eq!(selected.len(), 12, "warm reuse must not starve saved accounts");
}

#[test]
fn overdue_accounts_still_obey_cooldowns_and_explicit_identity() {
    let p = ServingAccountPolicy::default();
    let mut pool: Vec<_> = (1..=12).map(account).collect();
    for n in 1..=6 { drop(hold(&p, n)); }
    for cold in &mut pool[6..] { cold.cooling_until_ms = Some(10_000); }
    for _ in 0..120 {
        assert!(p.select_at(pool.clone(), None, 1000).unwrap().id.0.as_u128() <= 6);
    }
    let id = account(1).id.0.to_string();
    assert_eq!(p.select_at(pool.clone(), Some(&id), 10_001).unwrap().id, account(1).id);
    assert!(p.select_at(pool, None, 10_001).unwrap().id.0.as_u128() > 6);
}

#[test]
fn busy_warm_account_does_not_hide_unused_capacity() {
    let p = ServingAccountPolicy::default();
    let _active = hold(&p, 1);
    assert_eq!(p.select_at(vec![account(1), account(2)], None, 1000).unwrap().id,
        account(2).id);
}

#[test]
fn full_pool_spreads_work_without_a_seventh_resident() {
    let p = ServingAccountPolicy::default();
    let pool: Vec<_> = (1..=11).map(account).collect();
    let mut leases: Vec<_> = (1..=6).map(|n| hold(&p, n)).collect();
    for _ in 0..120 {
        let chosen = p.select_at(pool.clone(), None, 1000).unwrap();
        assert!(chosen.id.0.as_u128() <= 6);
        leases.push(hold(&p, chosen.id.0.as_u128()));
    }
    let snapshot = p.residents.snapshot().unwrap();
    assert_eq!(snapshot.occupied, 6);
    assert!(snapshot.residents.iter().all(|slot| slot.users == 21));
}

#[test]
fn cooling_idle_resident_can_make_room_for_an_unused_account() {
    let p = ServingAccountPolicy::default();
    let mut pool: Vec<_> = (1..=11).map(account).collect();
    drop(hold(&p, 1));
    let _active: Vec<_> = (2..=6).map(|n| hold(&p, n)).collect();
    pool[0].cooling_until_ms = Some(2000);
    let chosen = p.select_at(pool, None, 1000).unwrap();
    assert!(chosen.id.0.as_u128() > 6);
    assert_eq!(p.residents.snapshot().unwrap().occupied, 6);
}

#[test]
fn warmth_does_not_override_health_or_workspace_saturation() {
    let p = ServingAccountPolicy::default();
    drop(hold(&p, 1));
    let mut warm = account(1);
    warm.consecutive_failures = 2;
    assert_eq!(p.select_at(vec![warm, account(2)], None, 1000).unwrap().id,
        account(2).id);
    p.rotation().mark_saturated("team-1", 60_000);
    assert_eq!(p.select(vec![account(1), account(2)], None).unwrap().id,
        account(2).id);
}

#[test]
fn uncertain_resident_stays_excluded_even_when_idle() {
    let p = ServingAccountPolicy::default();
    let lease = hold(&p, 1);
    lease.mark_unconfirmed().unwrap();
    drop(lease);
    assert_eq!(p.select_at(vec![account(1), account(2)], None, 1000).unwrap().id,
        account(2).id);
    assert!(p.residents.snapshot().unwrap().residents[0].uncertain);
}
