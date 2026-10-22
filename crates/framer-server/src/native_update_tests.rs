use super::*;

async fn publish_refusal(state: &AppState) -> ManagedAccount {
    let mut admitted = acquire(state).await.unwrap();
    admitted.completion.start();
    admitted.completion.publish(state, &admitted.identity, &admitted.proxy,
        crate::native_control::HealthOutcome::RiskRefused).await.unwrap();
    admitted.pool.complete().unwrap();
    admitted.completion.complete().unwrap();
    state.state.get(id(1)).await.unwrap().unwrap()
}

fn assert_health(actual: &ManagedAccount, expected: &ManagedAccount) {
    assert_eq!(actual.cooling_until_ms, expected.cooling_until_ms);
    assert_eq!(actual.cooldown_step, expected.cooldown_step);
    assert_eq!(actual.consecutive_failures, expected.consecutive_failures);
    assert_eq!(actual.alias, expected.alias);
}

#[tokio::test]
async fn delayed_compat_credit_result_preserves_native_cooldown() {
    let (state, ports) = owned();
    ports.pause_usage.store(true, Ordering::SeqCst);
    let legacy_state = state.clone();
    let legacy = tokio::spawn(async move {
        crate::compat_forward(legacy_state, "openai", Some(id(1).0.to_string()), None,
            json!({"model":"gpt-4.1", "stream":false})).await
    });
    until(|| ports.usage_captured.load(Ordering::SeqCst) == 1).await;
    ports.records.lock().unwrap()[0].alias = "current account".into();
    let expected = publish_refusal(&state).await;
    ports.usage_gate.add_permits(1);
    let result = tokio::time::timeout(Duration::from_secs(2), legacy).await.unwrap().unwrap();
    assert!(matches!(result, Err(ApiError::AccountCooling { .. })));
    let current = state.state.get(id(1)).await.unwrap().unwrap();
    assert_health(&current, &expected);
    assert_eq!(current.credit.unwrap().available, 1000.0);
    assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
}

async fn refresh_pipeline_credit(state: AppState) -> ManagedAccount {
    let mut account = state.state.get(id(1)).await.unwrap().unwrap();
    let session = state.sessions.load(&account.framer_user_id).await.unwrap().unwrap();
    let deps = crate::Deps { client: state.client.as_ref(), browser: state.browser.as_ref(),
        sessions: state.sessions.as_ref(), state: state.state.as_ref(),
        account_updates: &state.serving_accounts.health_update };
    framer_core::credits::ensure_workspace(&deps, &mut account, &session).await.unwrap();
    account
}

#[tokio::test]
async fn delayed_pipeline_credit_refresh_preserves_native_cooldown() {
    let (state, ports) = owned();
    ports.pause_usage.store(true, Ordering::SeqCst);
    let task = tokio::spawn(refresh_pipeline_credit(state.clone()));
    until(|| ports.usage_captured.load(Ordering::SeqCst) == 1).await;
    let expected = publish_refusal(&state).await;
    ports.usage_gate.add_permits(1);
    let refreshed = task.await.unwrap();
    assert_health(&refreshed, &expected);
    assert_health(&state.state.get(id(1)).await.unwrap().unwrap(), &expected);
}

#[tokio::test]
async fn old_legacy_success_cannot_clear_new_native_refusal() {
    let (state, _) = owned();
    let old = state.state.get(id(1)).await.unwrap().unwrap();
    let expected = publish_refusal(&state).await;
    crate::record_account_health(&state, &old, &Ok::<(), String>(())).await;
    assert_health(&state.state.get(id(1)).await.unwrap().unwrap(), &expected);
}

#[tokio::test]
async fn credit_update_rejects_changed_account_scope_without_writing() {
    let (state, ports) = owned();
    let old = state.state.get(id(1)).await.unwrap().unwrap();
    ports.records.lock().unwrap()[0].current_team = Some(TeamId("replacement-team".into()));
    let result = framer_core::pipeline::update_account(state.state.as_ref(),
        &state.serving_accounts.health_update, &old, |current| current.credit = None).await;
    assert!(result.is_err());
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
}
