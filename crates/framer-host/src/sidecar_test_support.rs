use super::*;
use std::time::Duration;

pub(super) async fn blocked_input_bridge() -> (SidecarBridge, mpsc::Receiver<u64>) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/fixtures/sidecar_blocked_input.py");
    let mut child = Command::new("python3")
        .arg(fixture).stdin(Stdio::piped()).stdout(Stdio::piped())
        .kill_on_drop(true).spawn().unwrap();
    // Keep stdout in the child after observing readiness. Its input stays unread.
    {
        let mut output = BufReader::new(child.stdout.as_mut().unwrap()).lines();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), output.next_line())
                .await.unwrap().unwrap().as_deref(),
            Some("ready"),
        );
    }
    let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
    let (cancel, cancellations) = mpsc::channel(1);
    let bridge = SidecarBridge::new("unused-fixture-state");
    *bridge.inner.lock().await = Some(Sidecar {
        child, stdin, pending, cancel, next_id: Arc::new(AtomicU64::new(0)),
        healthy: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        failure: Arc::new(StdMutex::new(None)),
        incarnation: uuid::Uuid::new_v4(),
    });
    (bridge, cancellations)
}
