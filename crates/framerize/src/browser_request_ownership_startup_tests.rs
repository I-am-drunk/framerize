use super::*;
use framer_core::operation_gate::OperationKind;
use framer_core::port::{Session, SessionStore};

struct NoSessions;
#[async_trait::async_trait]
impl SessionStore for NoSessions {
    async fn load(&self, _: &framer_core::ids::FramerUserId) -> Result<Option<Session>, String> {
        panic!("startup fixture must not read credentials")
    }
    async fn save(&self, _: &Session) -> Result<(), String> { panic!("unexpected session write") }
    async fn delete(&self, _: &framer_core::ids::FramerUserId) -> Result<(), String> {
        panic!("unexpected session deletion")
    }
}

fn state() -> AppState {
    // Empty in-memory state and lazy adapters. No browser or network work starts.
    AppState {
        operation_ownership: None,
        #[cfg(feature = "native-dispatch")]
        native_dispatch: None,
        client: Arc::new(framer_api::HttpFramerClient::new()),
        browser: Arc::new(framer_host::sidecar::SidecarBridge::new("/fixture-no-browser")),
        sessions: Arc::new(NoSessions),
        state: Arc::new(framer_store::SqliteStateStore::open(":memory:").unwrap()),
        gate: Arc::new(framer_core::gate::ConcurrencyGate::new()),
        wave: Arc::new(framer_core::wave::WaveGuard::new()),
        serving_accounts: Arc::new(framer_server::ServingAccountPolicy::from_config(
            framer_server::PoolConfig {
                eligible_accounts: framer_server::EligibleAccounts::Only(vec![]),
                ..Default::default()
            }).unwrap()),
    }
}

#[test]
fn browser_request_ownership_cli_is_explicit() {
    let cli = Cli::try_parse_from(["framerize", "daemon"]).unwrap();
    assert!(matches!(cli.cmd, Cmd::Daemon { browser_request_ownership: false, .. }));
    let cli = Cli::try_parse_from(["framerize", "daemon", "--browser-request-ownership"]).unwrap();
    assert!(matches!(cli.cmd, Cmd::Daemon { browser_request_ownership: true, .. }));
}

#[cfg(feature = "native-dispatch")]
#[test]
fn browser_request_ownership_rejects_offline_and_resident_modes() {
    for flag in ["--offline-worker-config", "--offline-routing-config", "--resident-control-socket"] {
        let error = Cli::try_parse_from(["framerize", "daemon", "--browser-request-ownership",
            flag, "/fixture-unused"]).err().unwrap();
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
    let error = Cli::try_parse_from(["framerize", "daemon", "--browser-request-ownership",
        "--resident-control-socket", "/fixture-socket", "--resident-v2-worker-config",
        "/fixture-unused"]).err().unwrap();
    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[tokio::test]
async fn browser_request_ownership_preserves_defaults_and_browser_state() {
    let original = state();
    let unchanged = configure_browser_request_ownership(original.clone(), false).unwrap();
    assert!(unchanged.operation_ownership.is_none());
    let owned = configure_browser_request_ownership(original.clone(), true).unwrap();
    assert!(owned.operation_ownership.is_some());
    #[cfg(feature = "native-dispatch")]
    assert!(owned.native_dispatch.is_none());
    assert!(Arc::ptr_eq(&original.client, &owned.client));
    assert!(Arc::ptr_eq(&original.browser, &owned.browser));
    assert!(Arc::ptr_eq(&original.sessions, &owned.sessions));
    assert!(Arc::ptr_eq(&original.state, &owned.state));
    assert!(Arc::ptr_eq(&original.gate, &owned.gate));
    assert!(Arc::ptr_eq(&original.wave, &owned.wave));
    assert!(Arc::ptr_eq(&original.serving_accounts, &owned.serving_accounts));
    assert!(configure_browser_request_ownership(owned, true).is_err());
}

#[tokio::test]
async fn browser_request_ownership_guards_startup_and_retains_host_barrier() {
    use std::time::Duration;
    let state = configure_browser_request_ownership(state(), true).unwrap();
    let ownership = state.operation_ownership.as_ref().unwrap();
    assert!(ownership.unmanaged_host_close_required());
    start_prewarm(&state).unwrap().unwrap().await.unwrap();
    let mut ticket = ownership.seal().unwrap();
    assert!(start_prewarm(&state).is_err());
    let result: Result<(), _> = framer_server::operation_ownership::run(Some(ownership),
        OperationKind::Background, async { panic!("sealed startup work ran") }).await;
    assert!(result.is_err());
    let proof = tokio::time::timeout(Duration::from_secs(1), ticket.wait_application())
        .await.unwrap().unwrap();
    ownership.check_application_drained(&proof).unwrap();
    drop(proof);
    assert!(tokio::time::timeout(Duration::from_millis(20), ticket.wait()).await.is_err());
    ticket.cancel_before_control().unwrap();
    start_prewarm(&state).unwrap().unwrap().await.unwrap();
}

#[cfg(feature = "native-dispatch")]
#[tokio::test]
async fn browser_request_ownership_preserves_normal_hello_routes() {
    use axum::{body::{to_bytes, Body}, http::{Method, Request, StatusCode}};
    use tower::ServiceExt;
    let state = configure_browser_request_ownership(state(), true).unwrap();
    assert!(state.native_dispatch.is_none());
    let ownership = state.operation_ownership.clone().unwrap();
    let app = framer_server::router(state, "/fixture-no-web");
    for path in ["/api/hello", "/anthropic/api/hello", "/openai/api/hello"] {
        for method in [Method::HEAD, Method::GET] {
            let request = Request::builder().method(method.clone()).uri(path)
                .body(Body::empty()).unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = to_bytes(response.into_body(), 16).await.unwrap();
            let expected: &[u8] = if method == Method::HEAD { b"" } else { b"ok" };
            assert_eq!(bytes.as_ref(), expected);
        }
    }
    let mut ticket = ownership.seal().unwrap();
    let proof = tokio::time::timeout(std::time::Duration::from_secs(1),
        ticket.wait_application()).await.unwrap().unwrap();
    ownership.check_application_drained(&proof).unwrap();
    drop(proof);
    ticket.cancel_before_control().unwrap();
}
