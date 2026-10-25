//! Typed post-workspace admission through the actual /api/run router.
//! Requires typed core 45fd55bf and Server mapping/wiring ff723aaa.
//! Source-only: all compilation and runtime qualification are owner-held.
use super::*;
use axum::{body::{to_bytes, Body}, http::{Request, StatusCode}, response::Response};
use crate::operation_ownership::ServingOwnership;
use framer_core::account::AccountState;
use framer_core::operation_gate::OperationGate;
use framer_core::pipeline::{InferenceOutcome, Usage};
use std::time::Duration;
use tower::ServiceExt;

const WAIT: Duration = Duration::from_secs(2);

fn request() -> Request<Body> {
    Request::builder().method("POST").uri("/api/run")
        .header("content-type", "application/json")
        .body(Body::from(json!({"account_id":account(1).id,"prompt":"inert capacity check",
            "model":"gpt-6-astra","chat_session":"capacity-fixture"}).to_string())).unwrap()
}

fn outcome() -> InferenceOutcome {
    InferenceOutcome { text: "capacity fixture result".into(),
        usage: Usage { input_tokens: 2, output_tokens: 3 },
        chat_session: "capacity-fixture".into(), credits_remaining: Some(95.0) }
}

fn records(ports: &FixturePorts) -> Value {
    serde_json::to_value(&*ports.records.lock().unwrap()).unwrap()
}

fn setup() -> (AppState, Arc<FixturePorts>, Arc<ServingOwnership>, OperationGate) {
    let (mut state, ports) = fixture(false);
    state.serving_accounts = Arc::new(policy(&[1, 2]));
    ports.records.lock().unwrap()[0].credit = Some(CreditSnapshot {
        available: 100.0, funds: 1, grants: 1, allocated: 100.0,
    });
    *ports.native_session.lock().unwrap() = Some(Some(Session {
        active_user: account(1).framer_user_id, cookie_header: "session=fixture-only".into(),
    }));
    *ports.run_reply.lock().unwrap() = Some(InferenceResult::Completed(outcome()));
    let (scope, gate) = ServingOwnership::new_with_host_factory(
        state.wave.clone(), 32, |gate| gate).unwrap();
    state.operation_ownership = Some(scope.clone());
    (state, ports, scope, gate)
}

async fn after_session(state: &AppState, ports: &FixturePorts,
    change: impl FnOnce(&mut Vec<ManagedAccount>)) -> (Response, Value, u64, u64) {
    // Cached credit avoids workspace I/O. The guard cannot load the current
    // record until this existing shared update lock is released.
    let lock = state.serving_accounts.health_update.lock().await;
    let app = crate::router(state.clone(), "/fixture-no-web");
    let task = tokio::spawn(app.oneshot(request()));
    tokio::time::timeout(WAIT, async {
        while ports.session_calls.load(Ordering::SeqCst) == 0 { tokio::task::yield_now().await; }
    }).await.expect("request did not load the selected session");
    assert_eq!(ports.session_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.client_calls.load(Ordering::SeqCst), 0);
    assert!(!task.is_finished());
    change(&mut ports.records.lock().unwrap());
    let expected = records(ports);
    let before = framer_core::rotation::now_ms();
    drop(lock);
    let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    (response, expected, before, framer_core::rotation::now_ms())
}

fn preserved(state: &AppState, ports: &FixturePorts, expected: &Value, called: bool) {
    assert_eq!(&records(ports), expected);
    assert_eq!(ports.session_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.client_calls.load(Ordering::SeqCst), usize::from(called));
    assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), 0);
    let selected = account(1);
    let calls = if called { vec![(selected.framer_user_id, selected.id, selected.current_project.unwrap())] }
        else { Vec::new() };
    assert_eq!(*ports.run_calls.lock().unwrap(), calls);
    assert!(ports.saved.lock().unwrap().is_empty());
    assert!(ports.registered_proxies.lock().unwrap().is_empty());
    let snapshot = state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(snapshot.occupied, 1);
    assert_eq!(snapshot.residents.len(), 1);
    let resident = &snapshot.residents[0];
    assert_eq!(resident.account, selected.id);
    assert_eq!(resident.users, 0);
    assert!(!resident.uncertain);
    assert!(!resident.close_started);
    assert_eq!(resident.replacement, None);
}

async fn finished(state: &AppState, scope: &ServingOwnership, gate: &OperationGate) {
    let mut ticket = scope.seal().unwrap();
    let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
    scope.check_application_drained(&proof).unwrap();
    drop(proof);
    assert_eq!(gate.stats().unwrap().active, 1);
    assert!(!gate.stats().unwrap().unconfirmed);
    assert!(scope.unmanaged_host_close_required());
    {
        let wait = ticket.wait();
        tokio::pin!(wait);
        assert!(futures::poll!(wait.as_mut()).is_pending());
    }
    ticket.cancel_before_control().unwrap();
    let request = Request::builder().uri("/api/hello").body(Body::empty()).unwrap();
    let response = tokio::time::timeout(WAIT,
        crate::router(state.clone(), "/fixture-no-web").oneshot(request)).await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap().as_ref(), b"ok");
    assert_eq!(gate.stats().unwrap().active, 1);
    assert!(!gate.stats().unwrap().unconfirmed);
}

#[tokio::test]
async fn api_run_typed_cooldown_preserves_deadline_and_bounds_http_retry() {
    for delay in [5_500, 120_500] { for non_ready in [false, true] { for consume in [false, true] {
        let (state, ports, scope, gate) = setup();
        let (response, expected, before, after) = after_session(&state, &ports, |pool| {
            pool[0].cooling_until_ms = Some(framer_core::rotation::now_ms() + delay);
            if non_ready { pool[0].state = AccountState::Exhausted; }
        }).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["content-type"], "application/json");
        let retry: u64 = response.headers()["retry-after"].to_str().unwrap().parse().unwrap();
        let deadline = expected[0]["cooling_until_ms"].as_u64().unwrap();
        let lower = deadline.saturating_sub(after).div_ceil(1000);
        let upper = deadline.saturating_sub(before).div_ceil(1000);
        assert!(lower > 0);
        if delay > 30_000 { assert_eq!(retry, 30); }
        else { assert!((lower..=upper).contains(&retry)); }
        if consume {
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            let remaining = body["error"]["cooldown_remaining_seconds"].as_u64().unwrap();
            assert!((lower..=upper).contains(&remaining));
            assert_eq!(retry, remaining.clamp(1, 30));
            assert_eq!(body, json!({"type":"error","error":{"type":"overloaded_error",
                "code":"framerize_account_cooling","message":format!(
                    "Framerize has no ready account for this request. Retry in {retry} seconds."),
                "retry_after_seconds":retry,"cooldown_remaining_seconds":remaining}}));
        } else { drop(response); }
        finished(&state, &scope, &gate).await;
        preserved(&state, &ports, &expected, false);
        assert!(ports.run_reply.lock().unwrap().is_some());
    } } }
}

#[tokio::test]
async fn api_run_typed_not_ready_refuses_after_session_without_peer_substitution() {
    for lifecycle in [AccountState::Disconnected, AccountState::Authenticating,
        AccountState::CoolingDown, AccountState::Exhausted, AccountState::Quarantined] {
        for consume in [false, true] {
            let (state, ports, scope, gate) = setup();
            let (response, expected, _, _) = after_session(&state, &ports, |pool| {
                pool[0].state = lifecycle.clone();
                pool[0].cooling_until_ms = Some(0);
            }).await;
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(response.headers()["content-type"], "text/plain; charset=utf-8");
            assert!(!response.headers().contains_key("retry-after"));
            if consume {
                assert_eq!(to_bytes(response.into_body(), 65536).await.unwrap().as_ref(),
                    b"account is not ready");
            } else { drop(response); }
            finished(&state, &scope, &gate).await;
            preserved(&state, &ports, &expected, false);
            assert!(ports.run_reply.lock().unwrap().is_some());
        }
    }
}

#[tokio::test]
async fn api_run_expired_current_deadline_keeps_ready_success() {
    for deadline in [None, Some(0)] { for consume in [false, true] {
        let (state, ports, scope, gate) = setup();
        let (response, expected, _, _) = after_session(&state, &ports, |pool| {
            pool[0].cooling_until_ms = deadline;
        }).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!response.headers().contains_key("retry-after"));
        if consume {
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(),
                serde_json::to_value(outcome()).unwrap());
        } else { drop(response); }
        finished(&state, &scope, &gate).await;
        preserved(&state, &ports, &expected, true);
        assert!(ports.run_reply.lock().unwrap().is_none());
    } }
}

#[derive(Clone, Copy)]
enum Changed { User, Device, Team, Project, Proxy, Removed }

#[tokio::test]
async fn api_run_changed_scope_and_missing_record_are_not_capacity_errors() {
    for changed in [Changed::User, Changed::Device, Changed::Team,
        Changed::Project, Changed::Proxy, Changed::Removed] {
        for consume in [false, true] {
            let (state, ports, scope, gate) = setup();
            let (response, expected, _, _) = after_session(&state, &ports, |pool| {
                pool[0].cooling_until_ms = Some(u64::MAX);
                pool[0].state = AccountState::Exhausted;
                match changed {
                    Changed::User => pool[0].framer_user_id = FramerUserId("changed-user".into()),
                    Changed::Device => pool[0].castle_device_id = "changed-device".into(),
                    Changed::Team => pool[0].current_team = None,
                    Changed::Project => pool[0].current_project = None,
                    Changed::Proxy => pool[0].proxy = Some("http://changed.invalid:8080".into()),
                    Changed::Removed => { pool.remove(0); }
                }
            }).await;
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            assert!(!response.headers().contains_key("retry-after"));
            if consume {
                let expected_error = if matches!(changed, Changed::Removed) {
                    "account not eligible: account removed before inference"
                } else { "account not eligible: account scope changed before inference" };
                assert_eq!(to_bytes(response.into_body(), 65536).await.unwrap().as_ref(), expected_error.as_bytes());
            } else { drop(response); }
            finished(&state, &scope, &gate).await;
            preserved(&state, &ports, &expected, false);
            assert!(ports.run_reply.lock().unwrap().is_some());
        }
    }
}

#[tokio::test]
async fn api_run_capacity_words_in_upstream_errors_do_not_change_their_category() {
    for error in ["account-cooling:120", "account is not ready"] { for consume in [false, true] {
        let (state, ports, scope, gate) = setup();
        *ports.run_error.lock().unwrap() = Some(error.into());
        let (response, expected, _, _) = after_session(&state, &ports, |_| {}).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!response.headers().contains_key("retry-after"));
        if consume {
            let expected_error = format!("framer: {error}");
            assert_eq!(to_bytes(response.into_body(), 65536).await.unwrap().as_ref(), expected_error.as_bytes());
        } else { drop(response); }
        finished(&state, &scope, &gate).await;
        preserved(&state, &ports, &expected, true);
        assert!(ports.run_error.lock().unwrap().is_none());
        assert!(ports.run_reply.lock().unwrap().is_some());
    } }
}

#[tokio::test]
async fn api_run_mismatched_session_keeps_noncapacity_refusal() {
    for consume in [false, true] {
        let (state, ports, scope, gate) = setup();
        *ports.native_session.lock().unwrap() = Some(Some(Session {
            active_user: account(2).framer_user_id, cookie_header: "session=fixture-only".into(),
        }));
        let expected = records(&ports);
        let response = tokio::time::timeout(WAIT,
            crate::router(state.clone(), "/fixture-no-web").oneshot(request())).await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!response.headers().contains_key("retry-after"));
        if consume {
            assert_eq!(to_bytes(response.into_body(), 65536).await.unwrap().as_ref(),
                b"account not eligible: workspace session owner mismatch");
        } else { drop(response); }
        finished(&state, &scope, &gate).await;
        preserved(&state, &ports, &expected, false);
        assert!(ports.run_reply.lock().unwrap().is_some());
    }
}
