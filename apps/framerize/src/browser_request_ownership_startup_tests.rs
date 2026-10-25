use super::*;
use framer_core::operation_gate::OperationKind;
use framer_core::port::{Session, SessionStore};

struct NoSessions;

#[tokio::test]
async fn occupied_session_store_refuses_startup_before_browser_construction() {
    struct Temporary(std::path::PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    let root = Temporary(std::env::temp_dir().join(format!(
        "framerize-store-startup-{}", uuid::Uuid::new_v4())));
    let store = framer_store::FileSessionStore::new(root.0.join(".sessions")).unwrap();
    let db = root.0.join("accounts.db");
    let result = build_state(db.to_str().unwrap(), "unused", root.0.to_str().unwrap(),
        "browser-construction-must-not-be-reached", None, Some("none"), None).await;
    match result {
        Err(error) => assert!(error.contains("session store ownership unavailable"), "{error}"),
        Ok(_) => panic!("startup accepted an occupied session store"),
    }
    drop(store);
    assert!(framer_store::FileSessionStore::new(root.0.join(".sessions")).is_ok());
}

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

pub(crate) fn state() -> AppState {
    // Empty in-memory state and lazy adapters. No browser or network work starts.
    AppState::new(
        Arc::new(framer_api::HttpFramerClient::new()),
        Arc::new(framer_host::sidecar::SidecarBridge::new("/fixture-no-browser")),
        Arc::new(NoSessions),
        Arc::new(framer_store::SqliteStateStore::open(":memory:").unwrap()),
        Arc::new(framer_server::ServingAccountPolicy::from_config(
            framer_server::PoolConfig {
                eligible_accounts: framer_server::EligibleAccounts::Only(vec![]),
                ..Default::default()
            }).unwrap()),
    )
}

#[test]
fn shutdown_tracking_does_not_need_an_experimental_flag() {
    let cli = Cli::try_parse_from(["framerize", "daemon"]).unwrap();
    assert!(matches!(cli.cmd, Cmd::Daemon { .. }));
}

#[tokio::test]
async fn browser_request_ownership_preserves_defaults_and_browser_state() {
    let original = state();
    let owned = configure_browser_request_ownership(original.clone()).unwrap();
    assert!(owned.operation_ownership.is_some());
    assert!(Arc::ptr_eq(&original.client, &owned.client));
    assert!(Arc::ptr_eq(&original.browser, &owned.browser));
    assert!(Arc::ptr_eq(&original.sessions, &owned.sessions));
    assert!(Arc::ptr_eq(&original.state, &owned.state));
    assert!(Arc::ptr_eq(&original.gate, &owned.gate));
    assert!(Arc::ptr_eq(&original.wave, &owned.wave));
    assert!(Arc::ptr_eq(&original.serving_accounts, &owned.serving_accounts));
    assert!(configure_browser_request_ownership(owned).is_err());
}

#[tokio::test]
async fn browser_request_ownership_guards_startup_and_retains_host_barrier() {
    use std::time::Duration;
    let state = configure_browser_request_ownership(state()).unwrap();
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
