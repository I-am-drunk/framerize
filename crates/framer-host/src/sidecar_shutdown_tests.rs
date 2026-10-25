use super::*;
use std::time::Duration;

async fn fixture(exit: u8) -> SidecarBridge {
    let mut child = Command::new("python3").args(["-I", "-B", "-c",
        "import json,sys; assert json.loads(sys.stdin.readline()) == {'cmd':'shutdown'}; sys.exit(int(sys.argv[1]))",
        &exit.to_string()])
        .stdin(Stdio::piped()).stdout(Stdio::null()).kill_on_drop(true).spawn().unwrap();
    let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    let (cancel, _) = mpsc::channel(1);
    let bridge = SidecarBridge::new("/fixture-unused");
    *bridge.inner.lock().await = Some(Sidecar {
        child, stdin, cancel, pending: Arc::new(StdMutex::new(HashMap::new())),
        healthy: Arc::new(AtomicBool::new(true)),
        next_id: Arc::new(AtomicU64::new(0)), incarnation: uuid::Uuid::new_v4(),
    });
    bridge
}

#[tokio::test]
async fn shutdown_reaps_owned_host_and_never_restarts_it() {
    let bridge = fixture(0).await;
    tokio::time::timeout(Duration::from_secs(3), bridge.shutdown()).await.unwrap().unwrap();
    assert!(bridge.inner.lock().await.is_none());
    bridge.shutdown().await.unwrap();
    assert!(bridge.sidecar().await.unwrap_err().contains("shutting down"));
}

#[tokio::test]
async fn nonzero_host_exit_is_not_confirmed_cleanup() {
    let bridge = fixture(1).await;
    let error = tokio::time::timeout(Duration::from_secs(3), bridge.shutdown())
        .await.unwrap().unwrap_err();
    assert!(error.contains("shutdown failed"));
    assert!(bridge.inner.lock().await.is_some());
    assert!(bridge.sidecar().await.is_err());
}

#[tokio::test]
async fn shutdown_refuses_active_replies_without_writing_or_losing_child() {
    let bridge = fixture(0).await;
    let (send, _receive) = oneshot::channel();
    let pending = bridge.inner.lock().await.as_ref().unwrap().pending.clone();
    pending.lock().unwrap().insert(1, send);
    assert!(bridge.shutdown().await.unwrap_err().contains("active requests"));
    assert!(bridge.inner.lock().await.as_mut().unwrap().child.try_wait().unwrap().is_none());
    pending.lock().unwrap().remove(&1);
    tokio::time::timeout(Duration::from_secs(3), bridge.shutdown()).await.unwrap().unwrap();
}
