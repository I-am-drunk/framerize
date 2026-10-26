use super::*;
use crate::CdpBridge;
use framer_core::port::BrowserBridge;
use std::time::Duration;
use tokio::{sync::oneshot, time::timeout};

#[tokio::test]
async fn successful_publication_transfers_child_without_cleanup() {
    let owner = StartupChildren::default();
    let child = tokio::process::Command::new("python3")
        .args(["-I", "-B", "-c", "import sys; sys.stdin.read(1)"])
        .stdin(std::process::Stdio::piped()).kill_on_drop(true).spawn().unwrap();
    let mut child = owner.track(AccountId::new(), child).into_child();
    let registered = owner.join().await;
    let still_running = child.try_wait().unwrap().is_none();
    let killed = child.start_kill();
    let reaped = timeout(Duration::from_secs(2), child.wait()).await;
    killed.unwrap();
    reaped.unwrap().unwrap();
    assert!(registered.is_empty());
    assert!(still_running, "publication killed the live instance's child");
}

#[tokio::test]
async fn failed_cleanup_keeps_its_route_while_other_exits_join() {
    let bridge = CdpBridge::new("unused", "/uncreated-cdp-failed-shutdown");
    let failed = AccountId::new();
    let other = AccountId::new();
    bridge.route_cell(&failed).await.lock().await.may_have_browser = true;
    bridge.route_cell(&other).await.lock().await.may_have_browser = true;
    bridge.startup_children.register(Some(failed), tokio::spawn(async { Err("fixture wait failure".into()) }));
    let (exit, exited) = oneshot::channel();
    bridge.startup_children.register(Some(other), tokio::spawn(async move {
        exited.await.unwrap();
        Ok(())
    }));
    assert!(timeout(Duration::from_millis(10), bridge.shutdown()).await.is_err());
    assert!(bridge.route_cell(&other).await.lock().await.may_have_browser);
    exit.send(()).unwrap();
    let first = timeout(Duration::from_secs(2), bridge.shutdown()).await.unwrap();
    assert!(first.is_err());
    assert!(bridge.route_cell(&failed).await.lock().await.may_have_browser);
    assert!(!bridge.route_cell(&other).await.lock().await.may_have_browser);
    assert_eq!(bridge.shutdown().await, first);
}

#[tokio::test]
async fn cancelled_cleanup_waiter_retains_the_exit_result() {
    let owner = StartupChildren::default();
    let account = AccountId::new();
    let (exit, exited) = oneshot::channel();
    owner.register(Some(account), tokio::spawn(async move {
        exited.await.unwrap();
        Ok(())
    }));
    assert!(timeout(Duration::from_millis(10), owner.join()).await.is_err());
    exit.send(()).unwrap();
    let outcome = timeout(Duration::from_secs(2), owner.join()).await.unwrap();
    assert_eq!(outcome, vec![(Some(account), Ok(()))]);
    assert_eq!(owner.join().await, outcome);
}

#[tokio::test]
async fn cancelled_shutdown_keeps_route_uncertain_until_exit() {
    let bridge = CdpBridge::new("unused", "/uncreated-cdp-shutdown");
    let account = AccountId::new();
    let route = bridge.route_cell(&account).await;
    route.lock().await.may_have_browser = true;
    let (exit, exited) = oneshot::channel();
    bridge.startup_children.register(Some(account), tokio::spawn(async move {
        exited.await.unwrap();
        Ok(())
    }));
    assert!(timeout(Duration::from_millis(10), bridge.shutdown()).await.is_err());
    assert!(route.lock().await.may_have_browser);
    exit.send(()).unwrap();
    timeout(Duration::from_secs(2), bridge.shutdown()).await.unwrap().unwrap();
    assert!(!route.lock().await.may_have_browser);
    bridge.shutdown().await.unwrap();
}
