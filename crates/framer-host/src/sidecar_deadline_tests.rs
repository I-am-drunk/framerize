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
    let (bridge, mut cancellations) = super::test_support::blocked_input_bridge().await;
    let (stdin, pending) = {
        let state = bridge.inner.lock().await;
        let worker = state.as_ref().unwrap();
        (worker.stdin.clone(), worker.pending.clone())
    };

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
