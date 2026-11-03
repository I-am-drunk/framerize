use super::*;
use crate::pool_control::select_account_observed;

#[tokio::test]
async fn memory_pressure_preserves_the_cause_when_no_account_is_eligible() {
    for case in 0..5 {
        let (state, ports) = constrained().await;
        let mut accounts = ports.records.lock().unwrap().clone();
        let mut excluded = Vec::new();
        match case {
            0 => accounts.iter_mut().for_each(|a| a.state = AccountState::Disconnected),
            1 => accounts.iter_mut().for_each(|a| a.cooling_until_ms = Some(u64::MAX)),
            2 => excluded = accounts.iter().map(|a| a.id.0.to_string()).collect(),
            3 => state.serving_accounts.mutations.lock().unwrap()
                .extend(accounts.iter().map(|a| a.id)),
            _ => accounts.clear(),
        }
        let expected = if excluded.is_empty() {
            state.serving_accounts.select(accounts.clone(), None)
        } else {
            state.serving_accounts.select_fresh(accounts.clone(), &excluded)
        }.unwrap_err().diagnostic();
        let calls = ports.events.lock().unwrap().len();
        let observed = select_account_observed(&state, accounts, None, &excluded).await;
        assert!(observed.cleanup_confirmed);
        assert_eq!(observed.result.err().unwrap().diagnostic(), expected);
        assert!(!expected.contains("Low available RAM"));
        assert_eq!(ports.events.lock().unwrap().len(), calls);
    }
}

async fn constrained() -> (AppState, Arc<Ports>) {
    let (mut state, ports) = fixture(6, 3);
    for account in ports.records.lock().unwrap().iter_mut() {
        account.consecutive_failures = 0;
    }
    drop(lease_account(&state, id(1)).await.unwrap());
    let warm = lease_account(&state, id(2)).await.unwrap();
    ports.castle_token(&id(2)).await.unwrap();
    drop(warm);
    state.runtime_mut().unwrap().memory_admission =
        Arc::new(crate::memory_admission::Admission::fixed(Some(256 * 1024 * 1024)));
    (state, ports)
}

#[tokio::test]
async fn pressure_uses_a_loaded_browser_instead_of_an_absent_reserved_slot() {
    let (state, ports) = constrained().await;
    for _ in 0..48 {
        let accounts = ports.records.lock().unwrap().clone();
        let result = select_account_observed(&state, accounts, None, &[]).await;
        assert!(result.cleanup_confirmed);
        assert_eq!(result.result.unwrap().account(), id(2));
    }
    assert_eq!(ports.live.lock().unwrap().len(), 1);
    assert_eq!(ports.mint_routes.lock().unwrap().len(), 1);
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn deferred_accounts_keep_their_turn_when_memory_recovers() {
    let (mut state, ports) = constrained().await;
    for _ in 0..48 {
        let accounts = ports.records.lock().unwrap().clone();
        drop(select_account_observed(&state, accounts, None, &[]).await.result.unwrap());
    }
    state.runtime_mut().unwrap().memory_admission =
        Arc::new(crate::memory_admission::Admission::fixed(Some(2 * 1024 * 1024 * 1024)));
    let accounts = ports.records.lock().unwrap().clone();
    let chosen = select_account_observed(&state, accounts, None, &[]).await.result.unwrap();
    assert_ne!(chosen.account(), id(2));
}

#[tokio::test]
async fn explicit_identity_is_not_replaced_under_memory_pressure() {
    let (state, ports) = constrained().await;
    let accounts = ports.records.lock().unwrap().clone();
    let selected = select_account_observed(&state, accounts,
        Some(&id(1).0.to_string()), &[]).await.result.unwrap();
    assert_eq!(selected.account(), id(1));
    assert_eq!(ports.live.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn loaded_accounts_still_obey_cooldowns_and_request_exclusions() {
    for cooling in [false, true] {
        let (state, ports) = constrained().await;
        if cooling {
            ports.records.lock().unwrap()[1].cooling_until_ms =
                Some(framer_core::rotation::now_ms() + 60_000);
        }
        let excluded = if cooling { vec![] } else { vec![id(2).0.to_string()] };
        let accounts = ports.records.lock().unwrap().clone();
        let result = select_account_observed(&state, accounts, None, &excluded).await;
        assert!(result.cleanup_confirmed);
        let Err(ApiError::ServingCapacity(cause)) = result.result else {
            panic!("expected a local memory deferral");
        };
        assert!(cause.contains("256.00 MiB"));
        assert!(cause.contains("1024.00 MiB"));
        assert_eq!(ports.mint_routes.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn mismatched_status_cannot_authorize_memory_constrained_selection() {
    let (state, ports) = constrained().await;
    *ports.status_mode.lock().unwrap() = StatusMode::WrongAccount;
    let accounts = ports.records.lock().unwrap().clone();
    let result = select_account_observed(&state, accounts, None, &[]).await;
    assert!(!result.cleanup_confirmed);
    assert!(matches!(result.result, Err(ApiError::PoolBusy(_))));
}

#[tokio::test]
async fn browser_loss_after_selection_does_not_start_a_replacement() {
    let (state, ports) = constrained().await;
    let lease = lease_account(&state, id(2)).await.unwrap();
    ports.live.lock().unwrap().remove(&id(2));
    let cause = state.memory_admission.browser_deferral().unwrap();
    let result = super::super::confirm_resident_status(ports.as_ref(), lease, Some(&cause)).await;
    assert!(result.cleanup_confirmed);
    assert!(matches!(result.result, Err(ApiError::ServingCapacity(_))));
    assert!(ports.live.lock().unwrap().is_empty());
    assert_eq!(ports.mint_routes.lock().unwrap().len(), 1);
}
