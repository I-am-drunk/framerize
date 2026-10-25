use super::*;
use std::time::Duration;

#[tokio::test]
async fn blocked_input_does_not_hold_the_process_lifecycle_lock() {
    let (bridge, _cancellations) = super::test_support::blocked_input_bridge().await;

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
