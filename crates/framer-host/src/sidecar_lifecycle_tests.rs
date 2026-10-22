use super::*;
use std::time::Duration;

#[tokio::test]
async fn blocked_input_does_not_hold_the_process_lifecycle_lock() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/fixtures/sidecar_blocked_input.py");
    let mut child = Command::new("python3")
        .arg(fixture)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), output.next_line())
            .await.unwrap().unwrap().as_deref(),
        Some("ready"),
    );
    let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
    let (cancel, _cancellations) = mpsc::channel(1);
    let bridge = SidecarBridge::new("unused-fixture-state");
    *bridge.inner.lock().await = Some(Sidecar {
        child, stdin, pending, cancel, next_id: Arc::new(AtomicU64::new(0)),
        capability: Arc::new(Mutex::new(None)),
        healthy: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        incarnation: uuid::Uuid::new_v4(),
        events: tokio::sync::broadcast::channel(512).0,
    });

    // The child announces readiness, then leaves its input pipe unread.
    // A payload larger than the pipe capacity cannot finish writing.
    let mut call = Box::pin(bridge.call("fixture", json!({"body": "x".repeat(2 * 1024 * 1024)})));
    assert!(futures::poll!(call.as_mut()).is_pending());
    tokio::task::yield_now().await;
    let lifecycle_available = tokio::time::timeout(
        Duration::from_millis(100), bridge.inner.lock(),
    ).await.is_ok();

    // Even on a failed assertion, release the caller and terminate only the
    // owned fixture child. kill_on_drop also covers an earlier test panic.
    drop(call);
    let mut state = bridge.inner.lock().await;
    let worker = state.as_mut().unwrap();
    worker.child.kill().await.unwrap();
    worker.child.wait().await.unwrap();
    assert!(lifecycle_available, "a full input pipe held the process lifecycle lock");
}
