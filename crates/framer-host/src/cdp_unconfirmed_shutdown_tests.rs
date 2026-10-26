//! Synthetic route state and local WebSocket peers only; no browser is started.
use super::*;
use std::time::Duration;
use tokio::time::timeout;

const UNCONFIRMED: &str = "could not confirm every owned browser exited cleanly";

async fn connected_instance(bridge: &CdpBridge, account: AccountId) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (socket, _) = timeout(Duration::from_secs(2), listener.accept()).await.unwrap().unwrap();
        let mut ws = timeout(Duration::from_secs(2), tokio_tungstenite::accept_async(socket))
            .await.unwrap().unwrap();
        let frame = timeout(Duration::from_secs(2), ws.next()).await.unwrap().unwrap().unwrap();
        assert!(frame.is_close(), "known CDP transport was not disconnected");
    });
    let (ws, _) = timeout(Duration::from_secs(2), connect_async(format!("ws://{address}")))
        .await.unwrap().unwrap();
    bridge.instances.lock().await.insert(account,
        Arc::new(Mutex::new(CdpInstance { child: None, ws, next_id: 0, pending: Default::default() })));
    bridge.route_cell(&account).await.lock().await.may_have_browser = true;
    peer
}

fn fixture(engine: EngineKind) -> (CdpBridge, PathBuf) {
    let root = std::env::temp_dir().join(format!("uncreated-cdp-close-{}", uuid::Uuid::new_v4()));
    (CdpBridge::with_engine("unused-cdp-close", &root, engine), root)
}

#[tokio::test]
async fn unused_owned_routes_allow_repeated_shutdown() {
    for engine in [EngineKind::FullChrome, EngineKind::HeadlessShell] {
        let (bridge, root) = fixture(engine);
        bridge.register_account_proxy(&AccountId::new(), Some("http://proxy.invalid:3128".into()))
            .await.unwrap();
        bridge.shutdown().await.unwrap();
        bridge.shutdown().await.unwrap();
        assert!(!root.exists());
    }
}

#[tokio::test]
async fn attached_route_does_not_claim_an_owned_process() {
    let (bridge, root) = fixture(EngineKind::Attach);
    let account = AccountId::new();
    bridge.route_cell(&account).await.lock().await.may_have_browser = true;
    bridge.shutdown().await.unwrap();
    bridge.shutdown().await.unwrap();
    assert!(bridge.route_cell(&account).await.lock().await.may_have_browser);
    assert!(!root.exists());
}

#[tokio::test]
async fn unconfirmed_owned_start_prevents_clean_shutdown() {
    for engine in [EngineKind::FullChrome, EngineKind::HeadlessShell] {
        let (bridge, root) = fixture(engine);
        let account = AccountId::new();
        bridge.route_cell(&account).await.lock().await.may_have_browser = true;
        for _ in 0..2 {
            assert_eq!(bridge.shutdown().await.unwrap_err(), UNCONFIRMED);
            assert!(bridge.route_cell(&account).await.lock().await.may_have_browser);
        }
        assert!(bridge.instance(&account).await.err().unwrap().contains("shutting down"));
        assert!(!root.exists());
    }
}

#[tokio::test]
async fn known_instance_closes_even_when_another_start_is_unconfirmed() {
    let (bridge, root) = fixture(EngineKind::FullChrome);
    let unknown = AccountId::new();
    bridge.route_cell(&unknown).await.lock().await.may_have_browser = true;
    let peer = connected_instance(&bridge, AccountId::new()).await;
    let result = timeout(Duration::from_secs(3), bridge.shutdown()).await;
    let joined = timeout(Duration::from_secs(3), peer).await;
    joined.unwrap().unwrap();
    assert_eq!(result.unwrap().unwrap_err(), UNCONFIRMED);
    assert!(bridge.route_cell(&unknown).await.lock().await.may_have_browser);
    assert!(!root.exists());
}

#[tokio::test]
async fn confirmed_owned_route_allows_repeated_shutdown() {
    let (bridge, root) = fixture(EngineKind::FullChrome);
    let peer = connected_instance(&bridge, AccountId::new()).await;
    let first = timeout(Duration::from_secs(3), bridge.shutdown()).await;
    let joined = timeout(Duration::from_secs(3), peer).await;
    joined.unwrap().unwrap();
    first.unwrap().unwrap();
    bridge.shutdown().await.unwrap();
    assert!(bridge.instances.lock().await.is_empty());
    assert!(!root.exists());
}
