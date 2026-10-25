use super::*;

async fn resident_snapshot(state: &AppState) -> serde_json::Value {
    use tower::ServiceExt;
    let mut inspection = state.clone();
    inspection.operation_ownership = None;
    let request = axum::http::Request::builder().uri("/api/pool")
        .body(axum::body::Body::empty()).unwrap();
    let response = framer_server::router(inspection, "fixture-no-web-root")
        .oneshot(request).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["residency"].clone()
}

#[tokio::test]
async fn shared_store_panic_retains_panicking_and_already_held_accounts() {
    shared_store_loss(false).await;
}

#[tokio::test]
async fn cancelled_store_reader_stays_unconfirmed_after_sibling_worker_loss() {
    shared_store_loss(true).await;
}

#[tokio::test]
async fn ordinary_store_errors_and_missing_sessions_complete_cleanly() {
    for failure in [2, 3] {
        let (state, ports, spec, sessions) = fixture();
        ports.load_failure.store(failure, Ordering::SeqCst);
        let outcome = finish(start(state.clone(), &ports, spec, &sessions)).await;
        assert_eq!(outcome.failure, Some(FailureKind::Authorization));
        assert!(outcome.cleanup_confirmed && outcome.response.is_none());
        assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
        let snapshot = resident_snapshot(&state).await;
        assert!(snapshot["residents"].as_array().unwrap().iter().all(|slot|
            slot["users"].as_u64().unwrap() == 0 && slot["uncertain"] == false));
        sessions.shutdown(&state).await.unwrap();
    }
}

async fn shared_store_loss(cancel_first: bool) {
    let (mut state, ports, spec, sessions) = fixture();
    state.gate = Arc::new(framer_core::gate::ConcurrencyGate::with_config(
        framer_core::gate::GateConfig { initial: 2, min: 2, max: 2,
            recover_ms: 60_000, relax_ms: 60_000, min_interval_ms: 0 }));
    let other = sibling(&ports, &spec);
    let (first_id, second_id) = (spec.identity.account, other.identity.account);
    ports.read.try_acquire().unwrap().forget();
    let first = start(state.clone(), &ports, spec, &sessions);
    wait(&ports.loading).await;
    if cancel_first { first.cancel(); }
    assert!(first.cleanup.clone().now_or_never().is_none());
    ports.load_failure.store(1, Ordering::SeqCst);
    let second = start(state.clone(), &ports, other, &sessions);
    let second = finish(second).await;
    let first = finish(first).await;
    assert_retained(&state, first_id, &first).await;
    assert_retained(&state, second_id, &second).await;
    assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
    assert_eq!(*ports.events.lock().unwrap(), vec!["status", "load", "status", "load"]);
    assert!(sessions.bridge().shutdown().await.is_err());
    sessions.native().shutdown_idle().await.unwrap();
}

async fn assert_retained(state: &AppState, account: AccountId, outcome: &Outcome) {
    assert_eq!(outcome.failure, Some(FailureKind::Cleanup));
    assert!(!outcome.cleanup_confirmed && outcome.response.is_none());
    assert!(outcome.evidence.is_none());
    let snapshot = resident_snapshot(state).await;
    let slot = snapshot["residents"].as_array().unwrap().iter()
        .find(|slot| slot["account"] == serde_json::json!(account)).unwrap();
    assert!(slot["uncertain"] == true && slot["users"].as_u64().unwrap() > 0);
}

fn sibling(ports: &Ports, original: &Specification) -> Specification {
    let mut account = ports.account.lock().unwrap().clone();
    account.id = AccountId::new();
    let mut identity = original.identity.clone();
    identity.account = account.id;
    ports.others.lock().unwrap().insert(account.id, account);
    Specification { identity, route: original.route, payload: original.payload.clone(),
        context: AttemptContext {
            feature_category: original.context.feature_category.clone(),
            feature_name: original.context.feature_name.clone(),
            step: original.context.step.clone(), turn_id: original.context.turn_id.clone(),
            user_prompt: original.context.user_prompt.clone(),
        }, deadline: original.deadline, token_limit: original.token_limit }
}
