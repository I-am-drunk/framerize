use super::*;
use std::time::Duration;

#[tokio::test]
async fn deadline_includes_waiting_for_the_lifecycle_lock() {
    let bridge = SidecarBridge::new("unused-fixture-state");
    let _lifecycle = bridge.inner.lock().await;
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        bridge.call_with_timeout("fixture", json!({}), Duration::from_millis(100)),
    ).await.expect("the request deadline did not include lifecycle lock acquisition");
    assert!(result.unwrap_err().starts_with("sidecar timeout after "));
}

#[tokio::test]
async fn deadline_includes_blocked_input_and_releases_pending_reply() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/fixtures/sidecar_blocked_input.py");
    let mut child = Command::new("python3")
        .arg(fixture)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), output.next_line())
            .await.unwrap().unwrap().as_deref(),
        Some("ready"),
    );
    let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
    let (cancel, mut cancellations) = mpsc::channel(1);
    let bridge = SidecarBridge::new("unused-fixture-state");
    *bridge.inner.lock().await = Some(Sidecar {
        child, stdin: stdin.clone(), pending: pending.clone(), cancel,
        next_id: Arc::new(AtomicU64::new(0)),
        healthy: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        incarnation: uuid::Uuid::new_v4(),
    });

    let result = tokio::time::timeout(
        Duration::from_secs(1),
        bridge.call_with_timeout(
            "fixture", json!({"body": "x".repeat(2 * 1024 * 1024)}),
            Duration::from_millis(100),
        ),
    ).await;
    let pending_released = pending.lock().unwrap().is_empty();
    let write_still_owned = stdin.try_lock().is_err();
    let cancellation = cancellations.try_recv();

    // Finish only our disposable worker, including on regression failure.
    let mut state = bridge.inner.lock().await;
    let worker = state.as_mut().unwrap();
    worker.child.kill().await.unwrap();
    worker.child.wait().await.unwrap();

    let error = result.expect("a blocked write escaped the request deadline")
        .unwrap_err();
    assert!(error.starts_with("sidecar timeout after "));
    assert!(pending_released, "timeout retained a pending reply");
    assert_eq!(cancellation.unwrap(), 1);
    assert!(write_still_owned, "timeout abandoned a partially written command");
}
