use super::*;
use framer_core::ids::AccountId;

// No store method or HTTP request is called. SQLite is in-memory only; the
// keyring store is a zero-sized value. Runtime validation must remain offline.
fn inputs() -> ServingStateInputs {
    ServingStateInputs {
        client: Arc::new(framer_api::HttpFramerClient::new()),
        sessions: Arc::new(framer_store::KeyringSessionStore),
        state: Arc::new(framer_store::SqliteStateStore::open(":memory:").unwrap()),
        gate: Arc::new(ConcurrencyGate::new()),
        wave: Arc::new(WaveGuard::new()),
        serving_accounts: Arc::new(ServingAccountPolicy::parse(
            Some("00000000-0000-4000-8000-000000000001")).unwrap()),
    }
}

fn fresh_root() -> PathBuf {
    std::env::temp_dir().join(format!("retirable-state-{}", uuid::Uuid::new_v4()))
}

#[tokio::test]
async fn construction_preserves_supplied_services_policy_and_guards() {
    let root = fresh_root();
    assert!(!root.exists());
    let input = inputs();
    let expected = (input.client.clone(), input.sessions.clone(), input.state.clone(),
        input.gate.clone(), input.wave.clone(), input.serving_accounts.clone());
    let assembly = RetirableServingState::new(&root, input, 16).unwrap();
    let state = assembly.state();
    assert!(Arc::ptr_eq(&state.client, &expected.0));
    assert!(Arc::ptr_eq(&state.sessions, &expected.1));
    assert!(Arc::ptr_eq(&state.state, &expected.2));
    assert!(Arc::ptr_eq(&state.gate, &expected.3));
    assert!(Arc::ptr_eq(&state.wave, &expected.4));
    assert!(Arc::ptr_eq(&state.serving_accounts, &expected.5));
    assert_eq!(state.serving_accounts.configured_count(), Some(1));
    assert!(state.native_dispatch.is_none());
    assert!(Arc::ptr_eq(&state.browser, &assembly.host.browser()));
    assert!(Arc::ptr_eq(state.operation_ownership.as_ref().unwrap(),
        &assembly.host.ownership()));
    assert!(!root.exists());
}

#[tokio::test]
async fn cloned_app_state_and_retirement_use_the_same_host_and_gate() {
    let root = fresh_root();
    assert!(!root.exists());
    let assembly = RetirableServingState::new(&root, inputs(), 16).unwrap();
    let state = assembly.state().clone();
    let owner = state.operation_ownership.as_ref().unwrap();
    let mut ticket = owner.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    let sealed = assembly.begin_retirement(&proof).await.unwrap();
    assert!(state.browser.castle_token(&AccountId::new()).await.is_err());
    let joined = sealed.wait_local_tasks().await.unwrap();
    assert_eq!(joined.task_count(), 0);
    assert!(joined.leader_status().is_none());
    assert!(owner.unmanaged_host_close_required());
    drop(proof);
    assert!(matches!(ticket.wait().await, Err(GateError::StaleSeal)));
    assert!(!root.exists());
}
