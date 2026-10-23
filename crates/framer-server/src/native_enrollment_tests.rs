use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use framer_core::native_device::{DeviceEnrollment, DeviceFailure};
use tower::ServiceExt;

struct NoInference;
impl framer_core::native_dispatch::InvocationProvider for NoInference {
    fn policy(&self) -> framer_core::native_dispatch::Policy {
        panic!("enrollment must not request inference")
    }
    fn contains_account(&self, _: AccountId) -> bool {
        panic!("enrollment must not request inference")
    }
    fn prepare(&self, _: framer_core::native_dispatch::Preparation,
        _: tokio::sync::OwnedSemaphorePermit)
        -> BoxFuture<'_, Result<Box<dyn framer_core::native_dispatch::OwnedInvocation>,
            framer_core::native_dispatch::DispatchError>> {
        panic!("enrollment must not request inference")
    }
}

fn native_fixture() -> (AppState, Arc<FixturePorts>) {
    let (mut state, ports) = fixture(true);
    state.native_dispatch = Some(Arc::new(NoInference));
    *ports.native_session.lock().unwrap() = Some(Some(Session {
        active_user: account(2).framer_user_id, cookie_header: "fixture-only".into(),
    }));
    (state, ports)
}

fn body() -> Value {
    json!({"account_id": account(2).id,
        "expected_framer_user_id": account(2).framer_user_id.0,
        "observed_device_id": "observed-device"})
}

fn success(ports: &FixturePorts) {
    *ports.native_result.lock().unwrap() = Some(Ok(DeviceEnrollment {
        host_generation: "a".repeat(32), enrollment: uuid::Uuid::from_u128(7),
        cooldown_ms: 600_000,
    }));
}

async fn direct(state: AppState, body: Value) -> axum::response::Response {
    crate::native_enrollment::enroll(State(state),
        Json(serde_json::from_value(body).unwrap())).await
}

fn request() -> Request<Body> {
    Request::builder().method("POST").uri("/api/accounts/native/enroll")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body()).unwrap())).unwrap()
}

fn hello() -> Request<Body> {
    Request::builder().uri("/api/hello").body(Body::empty()).unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn browser_mode_and_bad_identity_refuse_before_io() {
    for variant in 0..3 {
        let (mut state, ports) = native_fixture();
        let mut input = body();
        match variant {
            0 => state.native_dispatch = None,
            1 => input["expected_framer_user_id"] = json!(" "),
            _ => input["observed_device_id"] = json!("invalid device"),
        }
        let response = direct(state, input).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        ports.assert_rejected_before_io(0);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn wrong_login_refuses_before_session_or_host_access() {
    let (state, ports) = native_fixture();
    let mut input = body();
    input["expected_framer_user_id"] = json!("another-user");
    assert_eq!(direct(state, input).await.status(), StatusCode::BAD_REQUEST);
    ports.assert_rejected_before_io(1);
}

#[tokio::test(flavor = "current_thread")]
async fn another_account_cannot_share_the_login_or_observed_device() {
    for same_user in [true, false] {
        let (state, ports) = native_fixture();
        {
            let mut records = ports.records.lock().unwrap();
            let user = records[1].framer_user_id.clone();
            if same_user { records[0].framer_user_id = user; }
            else { records[0].castle_device_id = "observed-device".into(); }
        }
        assert_eq!(direct(state, body()).await.status(), StatusCode::BAD_REQUEST);
        ports.assert_rejected_before_io(2);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn missing_or_mismatched_session_cannot_enroll_a_device() {
    for session in [None, Some(Session {
        active_user: FramerUserId("other-user".into()), cookie_header: "fixture-only".into(),
    })] {
        let (state, ports) = native_fixture();
        *ports.native_session.lock().unwrap() = Some(session);
        assert_eq!(direct(state, body()).await.status(), StatusCode::BAD_REQUEST);
        assert_eq!(ports.session_calls.load(Ordering::SeqCst), 1);
        assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
        assert_eq!(ports.state_writes.load(Ordering::SeqCst), 0);
        assert_eq!(ports.client_calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn existing_management_owner_prevents_enrollment() {
    let (state, ports) = native_fixture();
    let _owner = crate::pool_control::management(&state, account(2).id, false).await.unwrap();
    assert_eq!(direct(state.clone(), body()).await.status(), StatusCode::SERVICE_UNAVAILABLE);
    ports.assert_rejected_before_io(0);
}

#[tokio::test(flavor = "current_thread")]
async fn successful_enrollment_publishes_only_the_observed_device() {
    let (state, ports) = native_fixture();
    let mut expected = ports.records.lock().unwrap()[1].clone();
    success(&ports);
    let response = direct(state, body()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 16384).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["cooldown_ms"], 600_000);
    assert_eq!(value.as_object().unwrap().len(), 4);
    expected.castle_device_id = "observed-device".into();
    assert_eq!(serde_json::to_value(&ports.records.lock().unwrap()[1]).unwrap(),
        serde_json::to_value(&expected).unwrap());
    assert_eq!(*ports.native_calls.lock().unwrap(),
        vec![(expected.id, expected.framer_user_id, "observed-device".into())]);
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), 1);
    assert_eq!(ports.client_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn refused_host_enrollment_does_not_change_the_account() {
    let (state, ports) = native_fixture();
    let before = serde_json::to_value(&*ports.records.lock().unwrap()).unwrap();
    *ports.native_result.lock().unwrap() = Some(Err(DeviceFailure::Rejected));
    assert_eq!(direct(state, body()).await.status(), StatusCode::BAD_REQUEST);
    assert_eq!(serde_json::to_value(&*ports.records.lock().unwrap()).unwrap(), before);
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn discarded_completed_enrollment_body_keeps_admission_open() {
    let (state, ports) = native_fixture();
    success(&ports);
    let state = crate::operation_ownership::with_owned_operations(state, 16).unwrap();
    let app = crate::router(state, "/nonexistent-fixture-web");
    let response = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    assert_eq!(app.oneshot(hello()).await.unwrap().status(), StatusCode::OK);
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_enrollment_completion_fences_further_admission() {
    let (state, ports) = native_fixture();
    *ports.native_result.lock().unwrap() = Some(Err(DeviceFailure::Unconfirmed));
    let state = crate::operation_ownership::with_owned_operations(state, 16).unwrap();
    let app = crate::router(state, "/nonexistent-fixture-web");
    let response = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    drop(response);
    assert_eq!(app.oneshot(hello()).await.unwrap().status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), 0);
}
