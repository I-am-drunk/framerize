use super::*;
use super::super::select_account_observed;

#[tokio::test]
async fn eleven_accounts_rotate_through_six_slots_without_extra_browsers() {
    let (state, ports) = balanced_fixture(6, 11);
    let mut leases = Vec::new();
    let mut used = HashSet::new();
    for n in 1..=6 {
        activated(&state, n).await;
        leases.push(Some(lease_account(&state, id(n)).await.unwrap()));
        used.insert(id(n));
    }
    for index in 0..5 {
        drop(leases[index].take());
        ports.records.lock().unwrap()[index].cooling_until_ms = Some(u64::MAX);
        let pool = ports.records.lock().unwrap().clone();
        let result = select_account_observed(&state, pool, None, &[]).await;
        assert!(result.cleanup_confirmed);
        let lease = result.result.unwrap();
        assert!(used.insert(lease.account()));
        ports.castle_token(&lease.account()).await.unwrap();
        leases.push(Some(lease));
    }
    assert_eq!(used.len(), 11);
    assert_eq!(ports.max_live.load(Ordering::SeqCst), 6);
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 6);
}

fn balanced_fixture(capacity: usize, saved: u128) -> (AppState, Arc<Ports>) {
    let (state, ports) = fixture(capacity, saved);
    for account in ports.records.lock().unwrap().iter_mut() {
        account.consecutive_failures = 0;
        account.cooldown_step = 0;
    }
    (state, ports)
}

#[tokio::test]
async fn slow_browser_inspection_does_not_hold_the_selection_lock() {
    let (state, ports) = balanced_fixture(2, 2);
    activated(&state, 1).await;
    let pool = ports.records.lock().unwrap().clone();
    ports.hold_status.store(true, Ordering::SeqCst);
    let mut first = Box::pin(select_account_observed(&state, pool.clone(), None, &[]));
    assert!(futures::poll!(first.as_mut()).is_pending());
    assert_eq!(ports.status_captured.load(Ordering::SeqCst), 1);
    assert_eq!(slot(&state, id(1)).users, 1);
    let second = tokio::time::timeout(Duration::from_secs(1),
        select_account_observed(&state, pool, None, &[])).await.unwrap();
    assert!(second.cleanup_confirmed);
    assert_eq!(second.result.as_ref().unwrap().account(), id(2));
    ports.status_gate.add_permits(1);
    let first = first.await;
    assert!(first.cleanup_confirmed);
    assert_eq!(first.result.as_ref().unwrap().account(), id(1));
}

#[tokio::test]
async fn simultaneous_requests_reserve_different_accounts_before_browser_io() {
    for route in [Route::Free, Route::Native] {
        let (state, ports) = balanced_fixture(2, 3);
        activated(&state, 1).await;
        ports.pause_get.store(true, Ordering::SeqCst);
        let mut first = Box::pin(handler_response(&state, route, true));
        assert!(futures::poll!(first.as_mut()).is_pending());
        assert_eq!(ports.get_captured.load(Ordering::SeqCst), 1);
        let mut second = Box::pin(handler_response(&state, route, true));
        assert!(futures::poll!(second.as_mut()).is_pending());
        ports.get_gate.add_permits(1);
        let (first, second) = tokio::join!(first, second);
        let (first, second) = (first.unwrap(), second.unwrap());
        assert_eq!(first.status(), 200);
        assert_eq!(second.status(), 200);
        until(|| ports.request_started.load(Ordering::SeqCst) == 2).await;
        let used: HashSet<_> = ports.observed_chats.lock().unwrap().iter()
            .map(|(account, _)| *account).collect();
        assert_eq!(used.len(), 2);
        assert!(used.contains(&id(1)));
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 2);
        ports.request_gate.add_permits(4);
        let (first, second) = tokio::join!(
            to_bytes(first.into_body(), 65536), to_bytes(second.into_body(), 65536));
        first.unwrap();
        second.unwrap();
        until(|| state.serving_accounts.residents.snapshot().unwrap().residents.iter()
            .all(|slot| slot.users == 0)).await;
        assert!(ports.max_live.load(Ordering::SeqCst) <= 2);
    }
}
