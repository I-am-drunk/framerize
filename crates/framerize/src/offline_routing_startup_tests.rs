use super::*;

#[tokio::test]
async fn routing_legacy_startup_retains_owned_background_admission() {
    let (mut state, ports, _, _sessions) = fixture();
    state.serving_accounts = Arc::new(framer_server::ServingAccountPolicy::from_config(
        framer_server::PoolConfig { active_capacity: 1,
            eligible_accounts: framer_server::EligibleAccounts::Only(Vec::new()) }).unwrap());
    let seal = state.operation_ownership.as_ref().unwrap().seal().unwrap();
    assert!(crate::start_prewarm(&state).is_err());
    assert!(ports.events.lock().unwrap().is_empty());
    seal.cancel_before_control().unwrap();
    let task = crate::start_prewarm(&state).unwrap().expect("legacy prewarm was skipped");
    tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
    assert!(ports.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn routing_native_startup_and_admission_keep_the_real_sidecar_unstarted() {
    let (mut state, ports, spec, sessions) = fixture();
    let root = std::env::temp_dir().join(format!("routing-no-browser-{}", uuid::Uuid::new_v4()));
    state.browser = Arc::new(framer_host::sidecar::SidecarBridge::new(&root));
    let provider = provider(&state, &ports, &spec, &sessions, false);
    state.native_dispatch = Some(Arc::new(provider));
    let seal = state.operation_ownership.as_ref().unwrap().seal().unwrap();
    assert!(crate::start_prewarm(&state).unwrap().is_none());
    seal.cancel_before_control().unwrap();
    state.browser.register_account_proxy(&spec.identity.account, None).await.unwrap();
    let admission = native_control::admit(&state, spec.identity.clone(), "gpt-4.1", spec.deadline)
        .await.unwrap();
    admission.completion.complete().unwrap();
    drop((admission.capacity, admission.pool));
    assert_eq!(users(&state, spec.identity.account).await, 0);
    assert!(!root.exists(), "startup or admission created browser state");
    assert!(ports.events.lock().unwrap().is_empty());
}
