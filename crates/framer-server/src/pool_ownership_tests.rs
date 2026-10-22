use super::*;
use framer_core::operation_gate::{GateError, OperationKind};
use tower::ServiceExt;

fn owned(capacity: usize, saved: u128) -> (AppState, Arc<Ports>) {
    let (state, ports) = fixture(capacity, saved);
    (crate::operation_ownership::with_owned_operations(state, 32).unwrap(), ports)
}

async fn response(state: &AppState, route: Route, session: Option<&str>) -> axum::response::Response {
    let (path, payload) = match route {
        Route::Free => ("/anthropic/v1/messages", json!({"model":"claude-opus-5-5",
            "stream":true,"messages":[{"role":"user","content":"fixture"}]})),
        Route::Native => ("/openai/v1/responses", json!({"model":"gpt-6-astra","stream":true,"input":"fixture"})),
        Route::Compat => ("/paid/openai/v1/responses", json!({"model":"fixture","stream":true,"input":"fixture"})),
    };
    let mut req = axum::http::Request::builder().method("POST").uri(path)
        .header("content-type", "application/json").header("x-framerize-account", id(1).0.to_string());
    if let Some(session) = session { req = req.header("x-framerize-session", session); }
    let req = req.body(axum::body::Body::from(payload.to_string())).unwrap();
    tokio::time::timeout(Duration::from_secs(2), crate::router(state.clone(), "/fixture-no-web").oneshot(req))
        .await.unwrap().unwrap()
}

async fn drained(state: &AppState) {
    let scope = state.operation_ownership.as_ref().unwrap();
    let mut ticket = scope.seal().unwrap();
    let proof = tokio::time::timeout(Duration::from_secs(2), ticket.wait_application()).await.unwrap().unwrap();
    scope.check_application_drained(&proof).unwrap();
    drop(proof);
    assert!(tokio::time::timeout(Duration::from_millis(5), ticket.wait()).await.is_err());
    ticket.cancel_before_control().unwrap();
}

#[tokio::test]
async fn startup_pool_close_remains_owned_after_waiter_drop_and_seal() {
    let (state, ports) = owned(1, 2);
    activated(&state, 1).await;
    ports.records.lock().unwrap()[0].cooling_until_ms = Some(u64::MAX);
    ports.gate_close.store(true, Ordering::SeqCst);
    let warming = state.clone();
    let task = crate::operation_ownership::spawn(state.operation_ownership.as_ref(),
        OperationKind::Background, async move { super::super::prewarm_accounts(warming).await }).unwrap();
    until(|| ports.events.lock().unwrap().contains(&("close_started", id(1)))).await;
    drop(task);
    let scope = state.operation_ownership.as_ref().unwrap();
    let mut ticket = scope.seal().unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(5), ticket.wait_application()).await.is_err());
    assert!(slot(&state, id(1)).close_started);
    ports.records.lock().unwrap()[1].cooling_until_ms = Some(u64::MAX);
    ports.close_gate.add_permits(1);
    let proof = tokio::time::timeout(Duration::from_secs(2), ticket.wait_application()).await.unwrap().unwrap();
    scope.check_application_drained(&proof).unwrap();
    assert!(!ports.mint_routes.lock().unwrap().iter().any(|(account, _)| *account == id(2)));
    assert!(state.serving_accounts.residents.snapshot().unwrap().residents.iter().all(|slot| slot.users == 0));
}

#[tokio::test]
async fn owned_legacy_retry_keeps_replacement_residency_and_account_scoped_chats() {
    for route in [Route::Free, Route::Native] { for session in [None, Some("explicit-owned-chat")] {
        let (state, ports) = owned(1, 2);
        ports.fail_first_account.store(true, Ordering::SeqCst);
        let before = [crate::stable_session(&id(1)), crate::stable_session(&id(2))];
        let response = response(&state, route, session).await;
        until(|| ports.request_started.load(Ordering::SeqCst) == 2).await;
        assert_repick_chats(&ports, session, &before);
        assert!(slot(&state, id(2)).users > 0);
        assert!(matches!(parked(&state, 2).await, Err(ApiError::PoolBusy(_))));
        ports.request_gate.add_permits(2);
        until(|| ports.request_drops.load(Ordering::SeqCst) == 2).await;
        assert!(slot(&state, id(2)).users > 0);
        to_bytes(response.into_body(), 65536).await.unwrap();
        until(|| slot(&state, id(2)).users == 0).await;
        assert_eq!(ports.max_live.load(Ordering::SeqCst), 1);
        drained(&state).await;
    } }
}

#[tokio::test]
async fn legacy_router_cancellation_releases_local_slots_but_keeps_application_uncertainty() {
    for route in ROUTES { for after_frame in [false, true] {
        let (state, ports) = owned(1, 1);
        let response = response(&state, route, None).await;
        until(|| ports.request_started.load(Ordering::SeqCst) == 1).await;
        let admitted = if matches!(route, Route::Compat) {
            vec![("status", id(1)), ("status", id(1)), ("mint", id(1))]
        } else {
            vec![("status", id(1)), ("mint", id(1))]
        };
        assert_eq!(*ports.events.lock().unwrap(), admitted);
        let mut body = Box::pin(response.into_body().into_data_stream());
        if after_frame {
            ports.request_gate.add_permits(if matches!(route, Route::Compat) { 2 } else { 1 });
            ports.response_gate.add_permits(1);
            let frame = tokio::time::timeout(Duration::from_secs(2), body.next()).await.unwrap().unwrap().unwrap();
            assert!(!frame.is_empty());
        }
        assert_stream_residency(&state).await;
        drop(body);
        until(|| ports.request_drops.load(Ordering::SeqCst) == 1 && slot(&state, id(1)).users == 0).await;
        assert!(matches!(state.operation_ownership.as_ref().unwrap().seal(), Err(GateError::Unconfirmed)));
        assert_eq!(*ports.events.lock().unwrap(), admitted);
    } }
}

#[tokio::test]
async fn legacy_router_keeps_pool_and_application_owners_until_the_last_output_byte() {
    for route in ROUTES {
        let (state, ports) = owned(1, 1);
        let response = response(&state, route, None).await;
        assert_eq!(response.status(), 200);
        until(|| ports.request_started.load(Ordering::SeqCst) == 1).await;
        ports.request_gate.add_permits(2);
        until(|| ports.request_drops.load(Ordering::SeqCst) == 1).await;
        assert_stream_residency(&state).await;
        let scope = state.operation_ownership.as_ref().unwrap();
        let mut ticket = scope.seal().unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(5), ticket.wait_application()).await.is_err());
        ports.response_gate.add_permits(2);
        to_bytes(response.into_body(), 65536).await.unwrap();
        until(|| slot(&state, id(1)).users == 0).await;
        let proof = tokio::time::timeout(Duration::from_secs(2), ticket.wait_application()).await.unwrap().unwrap();
        scope.check_application_drained(&proof).unwrap();
        drop(proof);
        assert!(tokio::time::timeout(Duration::from_millis(5), ticket.wait()).await.is_err());
    }
}
