use super::*;

#[tokio::test]
async fn health_publication_preserves_newer_account_fields() {
    let (state, ports) = fixture(2, 2);
    let stale = account(1);
    ports.records.lock().unwrap()[0].alias = "updated during inference".into();
    let failure: Result<(), String> = Err("risk-refused".into());
    crate::record_account_health(&state, &stale, &failure).await;
    let current = state.state.get(id(1)).await.unwrap().unwrap();
    assert_eq!(current.alias, "updated during inference");
    assert_eq!(current.consecutive_failures, stale.consecutive_failures + 1);
}

#[tokio::test]
async fn credit_update_rejects_changed_account_scope_without_writing() {
    let (state, ports) = fixture(2, 2);
    let old = state.state.get(id(1)).await.unwrap().unwrap();
    ports.records.lock().unwrap()[0].current_team = Some(TeamId("replacement-team".into()));
    let result = framer_core::pipeline::update_account(state.state.as_ref(),
        &state.serving_accounts.health_update, &old, |current| current.credit = None).await;
    assert!(result.is_err());
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
}
