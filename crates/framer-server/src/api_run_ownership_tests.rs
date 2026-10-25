//! Actual-router completion fixtures. All sessions, replies and accounts are inert.
use super::*;
use axum::{body::{to_bytes, Body}, http::{Request, StatusCode}};
use crate::operation_ownership::ServingOwnership;
use framer_core::operation_gate::{GateError, OperationGate};
use framer_core::pipeline::{InferenceOutcome, Usage};
use framer_core::resident_pool::ResidentStatus;
use http_body::Body as _;
use std::time::Duration;
use tower::ServiceExt;

fn request(target: u128) -> Request<Body> {
    Request::builder().method("POST").uri("/api/run")
        .header("content-type", "application/json")
        .body(Body::from(json!({"account_id":account(target).id,
            "prompt":"inert owned request", "model":"gpt-6-astra",
            "chat_session":"inert-owned-chat"}).to_string())).unwrap()
}

fn outcome() -> InferenceOutcome {
    InferenceOutcome { text: "inert owned completion".into(),
        usage: Usage { input_tokens: 2, output_tokens: 3 },
        chat_session: "inert-owned-chat".into(), credits_remaining: Some(97.0) }
}

fn records(ports: &FixturePorts) -> Value {
    serde_json::to_value(&*ports.records.lock().unwrap()).unwrap()
}

fn prepared() -> (AppState, Arc<FixturePorts>, Arc<ServingOwnership>, OperationGate) {
    let (mut state, ports) = fixture(false);
    state.serving_accounts = Arc::new(policy(&[1, 2]));
    {
        let mut pool = ports.records.lock().unwrap();
        pool[0].credit = Some(CreditSnapshot {
            available: 100.0, funds: 1, grants: 1, allocated: 100.0,
        });
        pool[0].consecutive_failures = 3;
    }
    *ports.native_session.lock().unwrap() = Some(Some(Session {
        active_user: account(1).framer_user_id, cookie_header: "session=inert-owned".into(),
    }));
    let (ownership, gate) = ServingOwnership::new_with_host_factory(
        state.wave.clone(), 32, |gate| gate).unwrap();
    state.operation_ownership = Some(ownership.clone());
    (state, ports, ownership, gate)
}

fn assert_one_call(ports: &FixturePorts, before: &Value) {
    let target = account(1);
    assert_eq!(*ports.run_calls.lock().unwrap(), vec![(target.framer_user_id,
        target.id, target.current_project.unwrap())]);
    assert_eq!(ports.session_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.client_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), 0);
    assert_eq!(&records(ports), before); // Includes the untouched peer and both scopes.
    assert!(ports.saved.lock().unwrap().is_empty());
    assert!(ports.registered_proxies.lock().unwrap().is_empty());
}

async fn assert_completion(state: &AppState, ownership: &ServingOwnership,
    gate: &OperationGate, uncertain: bool) {
    assert_eq!(gate.stats().unwrap().active, 1);
    assert_eq!(gate.stats().unwrap().unconfirmed, uncertain);
    assert!(ownership.unmanaged_host_close_required());
    if uncertain { assert!(matches!(ownership.seal(), Err(GateError::Unconfirmed))); }
    else {
        let mut ticket = ownership.seal().unwrap();
        let proof = tokio::time::timeout(Duration::from_secs(2), ticket.wait_application())
            .await.unwrap().unwrap();
        ownership.check_application_drained(&proof).unwrap();
        drop(proof);
        {
            let waiting = ticket.wait();
            tokio::pin!(waiting);
            assert!(futures::poll!(waiting.as_mut()).is_pending());
        }
        ticket.cancel_before_control().unwrap();
    }
    let probe = Request::builder().uri("/api/hello").body(Body::empty()).unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2),
        crate::router(state.clone(), "/fixture-no-web").oneshot(probe)).await.unwrap().unwrap();
    assert_eq!(response.status(), if uncertain { StatusCode::SERVICE_UNAVAILABLE }
        else { StatusCode::OK });
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(body.as_ref(), if uncertain { b"operation admission closed".as_slice() }
        else { b"ok".as_slice() });
    assert_eq!(gate.stats().unwrap().active, 1); // Host retirement remains unproved.
    assert_eq!(gate.stats().unwrap().unconfirmed, uncertain);
    assert!(state.wave.check().is_none());
}

#[tokio::test]
async fn api_run_clean_success_preserves_output_and_releases_owned_request() {
    for consume in [false, true] {
        let (state, ports, ownership, gate) = prepared();
        let expected = serde_json::to_value(outcome()).unwrap();
        *ports.run_reply.lock().unwrap() = Some(InferenceResult::Completed(outcome()));
        let before = records(&ports);
        let response = tokio::time::timeout(Duration::from_secs(2),
            crate::router(state.clone(), "/fixture-no-web").oneshot(request(1)))
            .await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert!(!response.body().is_end_stream());
        if consume {
            let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), expected);
        } else { drop(response); }
        assert_one_call(&ports, &before);
        assert!(ports.run_reply.lock().unwrap().is_none());
        assert!(ports.run_error.lock().unwrap().is_none());
        let snapshot = state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.occupied, 1);
        assert_eq!(snapshot.residents.len(), 1);
        let resident = &snapshot.residents[0];
        assert_eq!(resident.account, account(1).id);
        assert_eq!(resident.users, 0);
        assert_eq!(resident.status, ResidentStatus::Idle);
        assert!(!resident.uncertain);
        assert!(!resident.close_started);
        assert_eq!(resident.replacement, None);
        assert_completion(&state, &ownership, &gate, false).await;
        assert_one_call(&ports, &before);
        assert_eq!(serde_json::to_value(state.serving_accounts.residents.snapshot().unwrap()).unwrap(),
            serde_json::to_value(snapshot).unwrap());
    }
}

#[tokio::test]
async fn api_run_disabled_account_refuses_before_work_without_fencing() {
    for consume in [false, true] {
        let (mut state, ports) = fixture(false);
        let (ownership, gate) = ServingOwnership::new_with_host_factory(
            state.wave.clone(), 32, |gate| gate).unwrap();
        state.operation_ownership = Some(ownership.clone());
        let before = records(&ports);
        let response = tokio::time::timeout(Duration::from_secs(2),
            crate::router(state.clone(), "/fixture-no-web").oneshot(request(2)))
            .await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["content-type"], "text/plain; charset=utf-8");
        assert!(!response.body().is_end_stream());
        if consume {
            let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
            assert_eq!(bytes.as_ref(),
                b"account is disabled by the pool configuration; its saved session is retained");
        } else { drop(response); }
        ports.assert_rejected_before_io(0);
        assert_eq!(records(&ports), before);
        assert!(ports.run_calls.lock().unwrap().is_empty());
        assert!(ports.saved.lock().unwrap().is_empty());
        assert!(ports.registered_proxies.lock().unwrap().is_empty());
        let snapshot = state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.occupied, 0);
        assert!(snapshot.residents.is_empty());
        assert_completion(&state, &ownership, &gate, false).await;
        ports.assert_rejected_before_io(0);
        assert_eq!(records(&ports), before);
    }
}

#[tokio::test]
async fn api_run_unknown_cleanup_preserves_error_and_fences_admission() {
    for consume in [false, true] {
        let (state, ports, ownership, gate) = prepared();
        *ports.run_error.lock().unwrap() = Some(
            "host-lifecycle-unconfirmed: inert api run".into());
        *ports.run_reply.lock().unwrap() = Some(InferenceResult::Completed(outcome()));
        let before = records(&ports);
        let response = tokio::time::timeout(Duration::from_secs(2),
            crate::router(state.clone(), "/fixture-no-web").oneshot(request(1)))
            .await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.headers()["content-type"], "text/plain; charset=utf-8");
        assert!(!response.body().is_end_stream());
        if consume {
            let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
            assert_eq!(bytes.as_ref(), b"framer: host-lifecycle-unconfirmed: inert api run");
        } else { drop(response); }
        assert_one_call(&ports, &before);
        assert!(ports.run_error.lock().unwrap().is_none());
        assert!(matches!(&*ports.run_reply.lock().unwrap(),
            Some(InferenceResult::Completed(reply)) if reply.text == "inert owned completion"));
        let snapshot = state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.occupied, 1);
        assert_eq!(snapshot.residents.len(), 1);
        let resident = &snapshot.residents[0];
        assert_eq!(resident.account, account(1).id);
        assert_eq!(resident.users, 0);
        assert_eq!(resident.status, ResidentStatus::Uncertain);
        assert!(resident.uncertain);
        assert!(!resident.close_started);
        assert_eq!(resident.replacement, None);
        assert_completion(&state, &ownership, &gate, true).await;
        assert_one_call(&ports, &before);
        assert_eq!(serde_json::to_value(state.serving_accounts.residents.snapshot().unwrap()).unwrap(),
            serde_json::to_value(snapshot).unwrap());
        assert!(matches!(crate::current_account(&state, account(1).id).await,
            Err(ApiError::PoolBusy(_))));
    }
}

#[derive(Clone, Copy, Debug)]
enum RunRejection { Malformed, MissingType, ReadFailure }

fn rejected_request(kind: RunRejection) -> Request<Body> {
    let body = match kind {
        RunRejection::Malformed => Body::from("{"),
        RunRejection::MissingType => Body::from("{}"),
        RunRejection::ReadFailure => Body::from_stream(futures::stream::iter([
            Err::<axum::body::Bytes, _>(std::io::Error::other("inert body read failure")),
        ])),
    };
    let mut request = Request::builder().method("POST").uri("/api/run");
    if !matches!(kind, RunRejection::MissingType) {
        request = request.header("content-type", "application/json");
    }
    request.body(body).unwrap()
}

fn assert_no_extraction_io(state: &AppState, ports: &FixturePorts, before: &Value) {
    ports.assert_rejected_before_io(0);
    assert_eq!(&records(ports), before);
    assert!(ports.run_calls.lock().unwrap().is_empty());
    assert!(ports.saved.lock().unwrap().is_empty());
    assert!(ports.registered_proxies.lock().unwrap().is_empty());
    let snapshot = state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(snapshot.occupied, 0);
    assert!(snapshot.residents.is_empty());
}

#[tokio::test]
async fn api_run_extraction_rejections_preserve_original_response_and_release_ownership() {
    use axum::{extract::FromRequest, response::IntoResponse};
    for kind in [RunRejection::Malformed, RunRejection::MissingType, RunRejection::ReadFailure] {
        for consume in [false, true] {
            let rejection = match Json::<crate::RunBody>::from_request(rejected_request(kind), &()).await {
                Err(rejection) => rejection,
                Ok(_) => panic!("invalid fixture body was accepted"),
            };
            let (expected, body) = rejection.into_response().into_parts();
            let expected_body = to_bytes(body, 4096).await.unwrap();
            assert!(!expected_body.is_empty());
            assert_eq!(expected.status, if matches!(kind, RunRejection::MissingType) {
                StatusCode::UNSUPPORTED_MEDIA_TYPE
            } else { StatusCode::BAD_REQUEST });
            let (mut state, ports) = fixture(false);
            let (ownership, gate) = ServingOwnership::new_with_host_factory(
                state.wave.clone(), 32, |gate| gate).unwrap();
            state.operation_ownership = Some(ownership.clone());
            let before = records(&ports);
            let response = tokio::time::timeout(Duration::from_secs(2),
                crate::router(state.clone(), "/fixture-no-web").oneshot(rejected_request(kind)))
                .await.unwrap().unwrap();
            assert_eq!(response.status(), expected.status, "{kind:?} {consume}");
            assert_eq!(response.headers().get("content-type"), expected.headers.get("content-type"));
            assert!(!response.body().is_end_stream());
            assert_eq!(response.body().size_hint().exact(), Some(expected_body.len() as u64));
            assert_no_extraction_io(&state, &ports, &before);
            if consume {
                let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
                assert_eq!(bytes, expected_body, "{kind:?}");
            } else { drop(response); }
            assert_completion(&state, &ownership, &gate, false).await;
            assert_no_extraction_io(&state, &ports, &before);
        }
    }
}
