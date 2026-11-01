use super::*;
use axum::{body::{to_bytes, Body}, http::Request};
use framer_core::port::CastleToken;
use std::sync::Mutex;
use tower::ServiceExt;
#[path = "permit_lifetime_fixture.rs"]
mod fixture;

struct History {
    checked: Mutex<Vec<AccountId>>,
    fail: bool,
    gate: Option<Arc<tokio::sync::Notify>>,
}

#[tokio::test]
async fn unread_readiness_reply_keeps_admission_available() {
    for fail in [false, true] {
        let mut f = fixture::Fixture::new();
        let history = Arc::new(History { checked: Mutex::new(Vec::new()), fail, gate: None });
        f.state.runtime_mut().unwrap().browser = history.clone();
        f.state = operation_ownership::with_owned_operations(f.state, 32).unwrap();
        let scope = f.state.operation_ownership.clone().unwrap();
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let response = app.oneshot(Request::builder().uri("/api/ready")
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), if fail { StatusCode::SERVICE_UNAVAILABLE }
            else { StatusCode::OK });
        drop(response);
        assert!(scope.admission_status().is_ok(), "completed readiness fenced admission");
        assert_eq!(history.checked.lock().unwrap().len(), f.accounts.len());
        assert!(f.loads.try_recv().is_err());
        f.no_call();
    }
}

#[tokio::test]
async fn ready_reply_is_bound_to_the_serving_process() {
    let mut f = fixture::Fixture::new();
    f.state.runtime_mut().unwrap().browser = Arc::new(History { checked: Mutex::new(Vec::new()), fail: false, gate: None });
    let app = crate::router(f.state, "/nonexistent-fixture-web");
    let response = app.oneshot(Request::builder().uri("/api/ready").body(Body::empty()).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-framerize-pid"], std::process::id().to_string());
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["ready"], true);
}

#[tokio::test]
async fn cancelled_readiness_waiter_retains_history_work_until_completion() {
    use std::time::Duration;
    let mut f = fixture::Fixture::new();
    let gate = Arc::new(tokio::sync::Notify::new());
    let history = Arc::new(History {
        checked: Mutex::new(Vec::new()), fail: false, gate: Some(gate.clone()),
    });
    f.state.runtime_mut().unwrap().browser = history.clone();
    f.state = operation_ownership::with_owned_operations(f.state, 32).unwrap();
    let scope = f.state.operation_ownership.clone().unwrap();
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let request = tokio::spawn(app.oneshot(Request::builder().uri("/api/ready")
        .body(Body::empty()).unwrap()));
    tokio::time::timeout(Duration::from_secs(1), async {
        while history.checked.lock().unwrap().is_empty() { tokio::task::yield_now().await; }
    }).await.unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    let mut drain = scope.seal().unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(10),
        drain.wait_application()).await.is_err());
    gate.notify_one();
    let proof = tokio::time::timeout(Duration::from_secs(1), drain.wait_application())
        .await.unwrap().unwrap();
    scope.check_application_drained(&proof).unwrap();
    assert!(f.loads.try_recv().is_err());
    f.no_call();
}

#[tokio::test]
async fn sealed_admission_does_not_start_history_work() {
    let mut f = fixture::Fixture::new();
    let history = Arc::new(History { checked: Mutex::new(Vec::new()), fail: false, gate: None });
    f.state.runtime_mut().unwrap().browser = history.clone();
    f.state = operation_ownership::with_owned_operations(f.state, 32).unwrap();
    let _ticket = f.state.operation_ownership.as_ref().unwrap().seal().unwrap();
    let app = crate::router(f.state, "/nonexistent-fixture-web");
    let response = app.oneshot(Request::builder().uri("/api/ready").body(Body::empty()).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(history.checked.lock().unwrap().is_empty());
}
#[async_trait::async_trait]
impl BrowserBridge for History {
    async fn check_account_history(&self, accounts: &[AccountId]) -> Result<(), String> {
        *self.checked.lock().unwrap() = accounts.to_vec();
        if let Some(gate) = &self.gate { gate.notified().await; }
        if self.fail {
            Err("rate history database is locked; Authorization: Bearer private-fixture-details".into())
        } else { Ok(()) }
    }
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        panic!("readiness must not mint")
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("readiness must not authenticate")
    }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> {
        panic!("readiness must not authenticate")
    }
}

#[tokio::test]
async fn live_http_does_not_mask_unavailable_account_history() {
    let mut f = fixture::Fixture::new();
    let history = Arc::new(History { checked: Mutex::new(Vec::new()), fail: true, gate: None });
    f.state.runtime_mut().unwrap().browser = history.clone();
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let request = |path| Request::builder().uri(path).body(Body::empty()).unwrap();
    let live = app.clone().oneshot(request("/api/health")).await.unwrap();
    assert_eq!(live.status(), StatusCode::OK);
    let response = app.oneshot(request("/api/ready")).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let reply: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply["ready"], false);
    assert_eq!(reply["reason"], "account-history-unavailable");
    assert!(reply["cause"].as_str().unwrap().contains("rate history database is locked"));
    assert!(!reply.to_string().contains("private-fixture-details"));
    assert_eq!(*history.checked.lock().unwrap(),
        f.accounts.iter().map(|account| account.id).collect::<Vec<_>>());
    assert!(f.loads.try_recv().is_err());
    f.no_call();
}
