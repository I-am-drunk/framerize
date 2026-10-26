use super::*;
use framer_core::{ids::AccountId, operation_gate::OperationKind,
    port::{BrowserBridge, CastleToken}};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::time::Duration;
use tokio::sync::{oneshot, Semaphore};

struct Browser {
    closes: AtomicUsize, fail: bool,
    runtime: std::sync::Mutex<std::sync::Weak<framer_server::AccountRuntime>>,
}
#[async_trait::async_trait]
impl BrowserBridge for Browser {
    async fn shutdown(&self) -> Result<(), String> {
        assert!(self.runtime.lock().unwrap().upgrade().is_some(),
            "runtime and private-store ownership must outlive host cleanup");
        self.closes.fetch_add(1, Ordering::SeqCst);
        if self.fail { Err("fixture close failure".into()) } else { Ok(()) }
    }
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> { panic!() }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> { panic!() }
    async fn auth_complete(&self, _: &AccountId, _: &str)
        -> Result<(String, serde_json::Value), String> { panic!() }
}

fn fixture(fail: bool) -> (AppState, Arc<Browser>) {
    let browser = Arc::new(Browser { closes: AtomicUsize::new(0), fail,
        runtime: Default::default() });
    let state = crate::browser_request_ownership_startup_tests::state_with_browser(browser.clone());
    let state = crate::configure_browser_request_ownership(state).unwrap();
    *browser.runtime.lock().unwrap() = Arc::downgrade(state.runtime());
    (state, browser)
}

const WAIT: Duration = Duration::from_secs(3);
fn addr() -> SocketAddr { "127.0.0.1:0".parse().unwrap() }

#[tokio::test]
async fn shutdown_seals_admission_and_waits_for_detached_work_before_close() {
    let (state, browser) = fixture(false);
    let ownership = state.operation_ownership.clone().unwrap();
    let release = Arc::new(Semaphore::new(0));
    let work = release.clone();
    let background = framer_server::operation_ownership::spawn(Some(&ownership),
        OperationKind::Background, async move { work.acquire().await.unwrap().forget(); }).unwrap();
    drop(background); // The application must retain work whose waiter disappeared.
    let (stopped, signal_seen) = oneshot::channel();
    let server = tokio::spawn(serve(state, "/fixture-no-web", addr(), async {
        stopped.send(()).unwrap();
    }));
    tokio::time::timeout(WAIT, signal_seen).await.unwrap().unwrap();
    assert!(framer_server::operation_ownership::spawn(Some(&ownership),
        OperationKind::Background, async {}).is_err());
    assert_eq!(browser.closes.load(Ordering::SeqCst), 0);
    assert!(!server.is_finished());
    release.add_permits(1);
    tokio::time::timeout(WAIT, server).await.unwrap().unwrap().unwrap();
    assert_eq!(browser.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn bind_failure_still_closes_the_host() {
    let (state, browser) = fixture(true);
    let occupied = tokio::net::TcpListener::bind(addr()).await.unwrap();
    let result = tokio::time::timeout(WAIT, serve(state, "/fixture-no-web",
        occupied.local_addr().unwrap(), std::future::pending())).await.unwrap();
    let error = result.unwrap_err();
    assert!(error.contains("fixture close failure"));
    assert!(error.contains("; shutdown:"));
    assert_eq!(browser.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn startup_cleanup_needs_no_listener_and_reports_host_failure() {
    for failed in [false, true] {
        let (state, browser) = fixture(failed);
        let result = tokio::time::timeout(WAIT, close(state)).await.unwrap();
        assert_eq!(result.is_err(), failed);
        assert_eq!(browser.closes.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn host_close_failure_is_reported_as_shutdown_failure() {
    let (state, browser) = fixture(true);
    let result = tokio::time::timeout(WAIT,
        serve(state, "/fixture-no-web", addr(), async {})).await.unwrap();
    assert_eq!(result.unwrap_err(), "fixture close failure");
    assert_eq!(browser.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unconfirmed_application_work_cannot_be_reported_as_clean_shutdown() {
    let (state, browser) = fixture(false);
    let task = framer_server::operation_ownership::spawn(state.operation_ownership.as_ref(),
        OperationKind::Background, std::future::pending::<()>()).unwrap();
    task.abort();
    assert!(task.await.is_err());
    let result = tokio::time::timeout(WAIT,
        serve(state, "/fixture-no-web", addr(), async {})).await.unwrap();
    assert!(result.unwrap_err().contains("Unconfirmed"));
    assert_eq!(browser.closes.load(Ordering::SeqCst), 0);
}
