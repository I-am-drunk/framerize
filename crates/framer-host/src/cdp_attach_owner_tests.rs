//! Admission fixtures stop at held route locks, before any browser or I/O.
use super::*;
use std::time::Duration;
use tokio::time::timeout;

const OTHER_ACCOUNT: &str = "attached browser is already assigned to another account";

fn fixture(engine: EngineKind) -> (CdpBridge, PathBuf) {
    let root = std::env::temp_dir().join(format!("uncreated-attach-{}", uuid::Uuid::new_v4()));
    (CdpBridge::with_engine("unused-endpoint", &root, engine), root)
}

async fn pending<T>(future: impl std::future::Future<Output = T>) {
    assert!(timeout(Duration::from_millis(10), future).await.is_err());
}

#[tokio::test]
async fn owned_browser_engines_keep_independent_account_admission() {
    for engine in [EngineKind::FullChrome, EngineKind::HeadlessShell] {
        let (bridge, root) = fixture(engine);
        let first = AccountId::new();
        let second = AccountId::new();
        let first_route = bridge.route_cell(&first).await;
        let second_route = bridge.route_cell(&second).await;
        let _first_hold = first_route.lock().await;
        let _second_hold = second_route.lock().await;
        pending(bridge.instance(&first)).await;
        pending(bridge.instance(&second)).await;
        assert!(bridge.instances.lock().await.is_empty());
        assert!(!root.exists());
    }
}

#[tokio::test]
async fn failed_attach_keeps_the_same_account_claim() {
    let (bridge, root) = fixture(EngineKind::Attach);
    let owner = AccountId::new();
    let other = AccountId::new();
    bridge.route_cell(&owner).await.lock().await.may_have_browser = true;
    let error = bridge.instance(&owner).await.err().expect("expected unconfirmed route");
    assert!(error.starts_with("cdp-lifecycle-unconfirmed:"));
    let other_route = bridge.route_cell(&other).await;
    let _other_hold = other_route.lock().await;
    let result = timeout(Duration::from_secs(1), bridge.instance(&other)).await
        .expect("failed attach released account ownership");
    assert_eq!(result.err().expect("owner changed after failure"), OTHER_ACCOUNT);
    assert!(!root.exists());
}

#[tokio::test]
async fn cancelled_attach_retains_owner_and_allows_same_account_retry() {
    let (bridge, root) = fixture(EngineKind::Attach);
    let owner = AccountId::new();
    let other = AccountId::new();
    let owner_route = bridge.route_cell(&owner).await;
    let other_route = bridge.route_cell(&other).await;
    let _owner_hold = owner_route.lock().await;
    let _other_hold = other_route.lock().await;
    pending(bridge.instance(&owner)).await;
    pending(bridge.instance(&owner)).await;
    let result = timeout(Duration::from_secs(1), bridge.instance(&other)).await
        .expect("cancelled attach released account ownership");
    assert_eq!(result.err().expect("owner changed after cancellation"), OTHER_ACCOUNT);
    assert!(bridge.instances.lock().await.is_empty());
    assert!(!root.exists());
}

#[tokio::test]
async fn another_attach_account_is_refused_before_browser_work() {
    let (bridge, root) = fixture(EngineKind::Attach);
    let first = AccountId::new();
    let second = AccountId::new();
    let first_route = bridge.route_cell(&first).await;
    let second_route = bridge.route_cell(&second).await;
    let _first_hold = first_route.lock().await;
    let _second_hold = second_route.lock().await;
    let mut starting = Box::pin(bridge.instance(&first));
    pending(starting.as_mut()).await;
    let result = timeout(Duration::from_secs(1), bridge.instance(&second)).await
        .expect("another account reached browser startup");
    assert_eq!(result.err().expect("shared attached browser"), OTHER_ACCOUNT);
    assert!(bridge.instances.lock().await.is_empty());
    assert!(!root.exists());
}
