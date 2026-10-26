use super::*;
use std::time::Duration;

#[tokio::test]
async fn history_readiness_uses_the_real_host_without_starting_a_browser() {
    let root = std::env::temp_dir().join(format!("history-readiness-{}", uuid::Uuid::new_v4()));
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/host.py").canonicalize().unwrap();
    let bridge = SidecarBridge::new(root.join(".sidecar")).with_script(script).unwrap();
    let account = AccountId(uuid::Uuid::from_u128(1));
    let result = bridge.check_account_history(&[account]).await;
    let history_created = root.join(".sidecar/camoufox/rate-history.sqlite3").is_file();
    let pid = bridge.inner.lock().await.as_ref().and_then(|host| host.child.id());
    let children = pid.and_then(|pid| std::fs::read_to_string(
        format!("/proc/{pid}/task/{pid}/children")).ok());
    let closed = tokio::time::timeout(Duration::from_secs(5), bridge.shutdown()).await;
    if let Some(host) = bridge.inner.lock().await.as_mut() {
        let _ = host.child.kill().await;
        let _ = host.child.wait().await;
    }
    let _ = std::fs::remove_dir_all(root);
    assert!(result.is_ok(), "{result:?}");
    assert!(history_created);
    assert!(children.is_some_and(|value| value.trim().is_empty()));
    assert!(matches!(closed, Ok(Ok(()))), "{closed:?}");
}
