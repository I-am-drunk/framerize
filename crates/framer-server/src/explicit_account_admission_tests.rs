//! Explicit admission checks using the existing inert endpoint ports.
use super::*;
use axum::{body::to_bytes, http::StatusCode};
use framer_core::account::AccountState;
use framer_core::resident_pool::{Acquire, ResidentStatus};

fn non_ready_states() -> [AccountState; 5] {
    [AccountState::Disconnected, AccountState::Authenticating,
        AccountState::CoolingDown, AccountState::Exhausted, AccountState::Quarantined]
}

fn records(ports: &FixturePorts) -> Value {
    serde_json::to_value(&*ports.records.lock().unwrap()).unwrap()
}

fn assert_unchanged_without_io(ports: &FixturePorts, before: &Value) {
    assert_eq!(records(ports), *before);
    assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.client_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.session_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), 0);
    assert!(ports.saved.lock().unwrap().is_empty());
    assert!(ports.registered_proxies.lock().unwrap().is_empty());
}

fn run_body() -> crate::RunBody {
    crate::RunBody { account_id: account(1).id, prompt: "fixture".into(),
        model: None, chat_session: None }
}

fn assert_no_request_lease_or_peer(state: &AppState) {
    let snapshot = state.serving_accounts.residents.snapshot().unwrap();
    assert!(snapshot.residents.iter().all(|slot|
        slot.account == account(1).id && slot.users == 0 && !slot.uncertain));
}

#[test]
fn explicit_non_ready_never_substitutes_a_ready_peer() {
    for lifecycle in non_ready_states() {
        for deadline in [None, Some(10_000)] {
            let (mut state, ports) = fixture(false);
            state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
            {
                let mut pool = ports.records.lock().unwrap();
                pool[0].state = lifecycle.clone();
                pool[0].cooling_until_ms = deadline;
            }
            let before = records(&ports);
            let pool = ports.records.lock().unwrap().clone();
            let requested = account(1).id.0.to_string();
            let result = state.serving_accounts.select_at(
                pool.clone(), Some(&requested), 10_000);
            assert!(matches!(result, Err(ApiError::ServingCapacity(_))),
                "{lifecycle:?} was not refused for lifecycle at {deadline:?}");
            // The peer is usable, but only an unbound request may select it.
            let peer = state.serving_accounts.select_at(pool, None, 10_000).unwrap();
            assert_eq!(peer.id, account(2).id);
            assert_unchanged_without_io(&ports, &before);
            ports.assert_rejected_before_io(0);
        }
    }
}

#[test]
fn explicit_ready_keeps_identity_despite_a_healthier_peer() {
    let (mut state, ports) = fixture(false);
    state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
    {
        let mut pool = ports.records.lock().unwrap();
        pool[0].consecutive_failures = 9;
        pool[0].cooldown_step = 4;
    }
    let before = records(&ports);
    let mut pool = ports.records.lock().unwrap().clone();
    pool.reverse();
    let requested = account(1).id.0.to_string();
    let chosen = state.serving_accounts.select_at(pool, Some(&requested), 10_000).unwrap();
    assert_eq!(serde_json::to_value(chosen).unwrap(), before[0]);
    assert_unchanged_without_io(&ports, &before);
    ports.assert_rejected_before_io(0);
}

#[test]
fn explicit_ready_preserves_deadline_and_reopens_exactly_at_expiry() {
    let (mut state, ports) = fixture(false);
    state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
    ports.records.lock().unwrap()[0].cooling_until_ms = Some(10_000);
    let before = records(&ports);
    let pool = ports.records.lock().unwrap().clone();
    let requested = account(1).id.0.to_string();
    for now in [8_001, 9_999] {
        let result = state.serving_accounts.select_at(pool.clone(), Some(&requested), now);
        assert!(matches!(result, Err(ApiError::AccountCooling { .. })));
        assert_unchanged_without_io(&ports, &before);
    }
    for now in [10_000, 10_001] {
        let chosen = state.serving_accounts.select_at(pool.clone(), Some(&requested), now).unwrap();
        assert_eq!(serde_json::to_value(chosen).unwrap(), before[0]);
        assert_unchanged_without_io(&ports, &before);
    }
    ports.assert_rejected_before_io(0);
}

#[tokio::test]
async fn api_run_non_ready_refuses_before_session_or_client_io() {
    for lifecycle in non_ready_states() {
        for deadline in [None, Some(0)] {
            let (mut state, ports) = fixture(false);
            state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
            {
                let mut pool = ports.records.lock().unwrap();
                pool[0].state = lifecycle.clone();
                pool[0].cooling_until_ms = deadline;
                pool[0].consecutive_failures = 3;
                pool[0].cooldown_step = 2;
            }
            let before = records(&ports);
            let result = crate::run(State(state.clone()), Json(run_body())).await;
            let response = result.unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
            assert_eq!(bytes.as_ref(),
                format!("account state is {lifecycle:?}; inference requires Ready").as_bytes());
            assert_unchanged_without_io(&ports, &before);
            assert_no_request_lease_or_peer(&state);
        }
    }
}

#[tokio::test]
async fn api_run_ready_preserves_active_deadline_before_io() {
    let (mut state, ports) = fixture(false);
    state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
    ports.records.lock().unwrap()[0].cooling_until_ms = Some(u64::MAX);
    let before = records(&ports);
    let result = crate::run(State(state.clone()), Json(run_body())).await;
    let response = result.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(body["error"]["code"], "framerize_account_cooling");
    assert_unchanged_without_io(&ports, &before);
    assert_no_request_lease_or_peer(&state);
}

#[tokio::test]
async fn api_run_ready_without_active_deadline_reaches_only_the_session_boundary() {
    for deadline in [None, Some(0)] {
        let (mut state, ports) = fixture(false);
        state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
        ports.records.lock().unwrap()[0].cooling_until_ms = deadline;
        // Leave inference unconfigured: this control must stop at the existing
        // missing-session boundary before it can reach the client.
        *ports.native_session.lock().unwrap() = Some(None);
        let before = records(&ports);
        let result = crate::run(State(state.clone()), Json(run_body())).await;
        let response = result.unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(bytes.as_ref(), framer_core::error::CoreError::NoSession.to_string().as_bytes());
        assert_eq!(ports.session_calls.load(Ordering::SeqCst), 1);
        assert_eq!(ports.client_calls.load(Ordering::SeqCst), 0);
        assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
        assert_eq!(ports.state_writes.load(Ordering::SeqCst), 0);
        assert_eq!(records(&ports), before);
        assert!(ports.saved.lock().unwrap().is_empty());
        assert!(ports.registered_proxies.lock().unwrap().is_empty());
        assert_no_request_lease_or_peer(&state);
    }
}

#[tokio::test]
async fn api_run_ready_completes_once_with_matched_identity_and_releases_lease() {
    use framer_core::pipeline::{InferenceOutcome, Usage};
    let (mut state, ports) = fixture(false);
    state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
    let target = account(1);
    {
        let mut pool = ports.records.lock().unwrap();
        pool[0].credit = Some(CreditSnapshot {
            available: 100.0, funds: 1, grants: 1, allocated: 100.0,
        });
        pool[0].consecutive_failures = 3;
    }
    *ports.native_session.lock().unwrap() = Some(Some(Session {
        active_user: target.framer_user_id.clone(), cookie_header: "session=inert-ready".into(),
    }));
    let outcome = InferenceOutcome {
        text: "inert ready completion".into(), usage: Usage { input_tokens: 2, output_tokens: 3 },
        chat_session: "inert-ready-chat".into(), credits_remaining: Some(97.0),
    };
    let expected = serde_json::to_value(&outcome).unwrap();
    *ports.run_reply.lock().unwrap() = Some(InferenceResult::Completed(outcome));
    let before = records(&ports);
    let result = crate::run(State(state.clone()), Json(run_body())).await.unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let actual: Value = serde_json::from_slice(&to_bytes(result.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(*ports.run_calls.lock().unwrap(), vec![(target.framer_user_id,
        target.id, target.current_project.unwrap())]);
    assert!(ports.run_reply.lock().unwrap().is_none());
    assert_eq!(ports.session_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.client_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), 0);
    assert_eq!(records(&ports), before);
    assert!(ports.saved.lock().unwrap().is_empty());
    assert!(ports.registered_proxies.lock().unwrap().is_empty());
    assert_no_request_lease_or_peer(&state);
    let snapshot = state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(snapshot.occupied, 1);
    assert_eq!(snapshot.residents[0].status, ResidentStatus::Idle);
}

#[tokio::test]
async fn ready_explicit_selection_and_run_require_a_complete_current_scope() {
    for missing_team in [false, true] {
        let (mut state, ports) = fixture(false);
        state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
        {
            let mut pool = ports.records.lock().unwrap();
            if missing_team { pool[0].current_team = None; }
            else { pool[0].current_project = None; }
        }
        let before = records(&ports);
        let requested = account(1).id.0.to_string();
        let pool = ports.records.lock().unwrap().clone();
        assert!(matches!(state.serving_accounts.select_at(pool, Some(&requested), 10_000),
            Err(ApiError::ServingCapacity(_))));
        let result = crate::run(State(state.clone()), Json(run_body())).await;
        let response = result.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(bytes.as_ref(), if missing_team {
            b"no current workspace is configured".as_slice()
        } else { b"no current project is configured".as_slice() });
        assert_unchanged_without_io(&ports, &before);
        assert_no_request_lease_or_peer(&state);
    }
}

#[tokio::test]
async fn ready_explicit_preserves_management_fence_until_owner_release() {
    let (mut state, ports) = fixture(false);
    state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
    ports.records.lock().unwrap()[0].cooling_until_ms = Some(0);
    let owner = crate::pool_control::management(&state, account(1).id, false).await.unwrap();
    let before = records(&ports);
    let requested = account(1).id.0.to_string();
    let pool = ports.records.lock().unwrap().clone();
    assert!(matches!(state.serving_accounts.select_at(pool.clone(), Some(&requested), 10_000),
        Err(ApiError::PoolBusy(_))));
    let result = crate::run(State(state.clone()), Json(run_body())).await;
    let response = result.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(body["error"]["code"], "framerize_pool_busy");
    assert!(state.serving_accounts.mutations.lock().unwrap().contains(&account(1).id));
    assert!(state.serving_accounts.residents.snapshot().unwrap().residents.is_empty());
    assert_unchanged_without_io(&ports, &before);
    drop(owner);
    assert!(!state.serving_accounts.mutations.lock().unwrap().contains(&account(1).id));
    let chosen = state.serving_accounts.select_at(pool, Some(&requested), 10_000).unwrap();
    assert_eq!(serde_json::to_value(chosen).unwrap(), before[0]);
    assert_unchanged_without_io(&ports, &before);
}

#[tokio::test]
async fn ready_explicit_preserves_unconfirmed_cleanup_after_lease_drop() {
    let (mut state, ports) = fixture(false);
    state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
    ports.records.lock().unwrap()[0].cooling_until_ms = Some(0);
    let lease = match state.serving_accounts.residents.try_acquire(account(1).id, None).unwrap() {
        Acquire::Lease(lease) => lease,
        Acquire::Evict(_) => panic!("empty fixture unexpectedly requires eviction"),
    };
    lease.mark_unconfirmed().unwrap();
    let mut owner = Some(lease);
    let before = records(&ports);
    let requested = account(1).id.0.to_string();
    for released in [false, true] {
        if released { drop(owner.take()); }
        let snapshot = state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.occupied, 1);
        assert_eq!(snapshot.residents[0].users, usize::from(!released));
        assert!(snapshot.residents[0].uncertain);
        let pool = ports.records.lock().unwrap().clone();
        assert!(matches!(state.serving_accounts.select_at(pool, Some(&requested), 10_000),
            Err(ApiError::PoolBusy(_))));
        assert!(matches!(crate::current_account(&state, account(1).id).await,
            Err(ApiError::PoolBusy(_))));
        let after = state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(serde_json::to_value(after).unwrap(), serde_json::to_value(snapshot).unwrap());
        assert_unchanged_without_io(&ports, &before);
    }
}

#[tokio::test]
async fn ready_explicit_preserves_closing_fence_and_unconfirmed_close() {
    let (mut state, ports) = fixture(false);
    state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
    let lease = match state.serving_accounts.residents.try_acquire(account(1).id, None).unwrap() {
        Acquire::Lease(lease) => lease,
        Acquire::Evict(_) => panic!("empty fixture unexpectedly requires eviction"),
    };
    drop(lease);
    let mut closing = state.serving_accounts.residents.begin_park(account(1).id).unwrap();
    closing.close_started().unwrap();
    let mut owner = Some(closing);
    let before = records(&ports);
    let requested = account(1).id.0.to_string();
    for status in [ResidentStatus::Closing, ResidentStatus::Uncertain] {
        if status == ResidentStatus::Uncertain { drop(owner.take()); }
        let snapshot = state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.occupied, 1);
        assert_eq!(snapshot.residents[0].status, status);
        let pool = ports.records.lock().unwrap().clone();
        assert!(matches!(state.serving_accounts.select_at(pool, Some(&requested), 10_000),
            Err(ApiError::PoolBusy(_))));
        assert!(matches!(crate::current_account(&state, account(1).id).await,
            Err(ApiError::PoolBusy(_))));
        let after = state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(serde_json::to_value(after).unwrap(), serde_json::to_value(snapshot).unwrap());
        assert_unchanged_without_io(&ports, &before);
    }
}

#[tokio::test]
async fn api_run_ready_keeps_existing_wave_backoff_before_io() {
    let (mut state, ports) = fixture(false);
    state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
    state.wave.note_refusal("fixture-a");
    state.wave.note_refusal("fixture-b");
    assert!(state.wave.check().is_some());
    let before = records(&ports);
    let result = crate::run(State(state.clone()), Json(run_body())).await;
    let response = result.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(body["error"]["code"], "framerize_cooling");
    assert!(state.wave.check().is_some());
    assert_unchanged_without_io(&ports, &before);
    assert_no_request_lease_or_peer(&state);
}

#[tokio::test]
async fn current_account_rechecks_lifecycle_after_ready_selection() {
    let (state, ports) = fixture(false);
    let requested = account(1).id.0.to_string();
    let pool = ports.records.lock().unwrap().clone();
    let selected = state.serving_accounts.select_at(pool, Some(&requested), 10_000).unwrap();
    ports.records.lock().unwrap()[0].state = AccountState::Quarantined;
    let before = records(&ports);
    assert!(matches!(crate::current_account(&state, selected.id).await,
        Err(ApiError::ServingCapacity(_))));
    assert_unchanged_without_io(&ports, &before);
    assert_no_request_lease_or_peer(&state);
}
