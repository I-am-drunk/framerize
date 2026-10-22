use super::*;
use framer_core::operation_gate::OperationGate;
use std::time::Duration;

#[tokio::test]
async fn malformed_session_error_never_contains_supplied_secret() {
    let bridge = SidecarBridge::new("unused-malformed-session");
    let secret = "synthetic-session-secret-without-equals";
    let error = bridge.seed_session(&AccountId::new(), secret).await.unwrap_err();
    assert_eq!(error, "bad cookie header");
    assert!(!error.contains(secret));
    assert!(bridge.inner.lock().await.is_none());
    assert!(bridge.routes.lock().await.is_empty());
}

#[tokio::test]
async fn unused_retirable_host_has_nonspawning_status_and_registration() {
    let (gate, _host) = OperationGate::new_with_legacy_host(8).unwrap();
    let (bridge, _control) = SidecarBridge::new_retirable("unused-local-status", gate);
    let account = AccountId::new();
    let status = bridge.inspect_browser(&account).await.unwrap();
    assert_eq!(status.state, framer_core::browser_lifecycle::BrowserAccountState::Absent);
    assert!(status.host_generation.is_none());
    assert!(bridge.close_observed_browser(&account, &status).await.unwrap()
        .confirms_absence(&status));
    bridge.register_proxy(&account, Some("http://proxy.invalid:8080".into())).await.unwrap();
    assert_eq!(bridge.route_snapshot(&account).await.unwrap().revision, 1);
    assert!(bridge.inner.lock().await.is_none());
}

#[tokio::test]
async fn sealed_existing_host_receives_no_pool_command_frames() {
    let (fixture, mut frames, reader) = super::proxy_tests::echo_bridge(None).await;
    let (gate, _host) = OperationGate::new_with_legacy_host(8).unwrap();
    let (bridge, control) = SidecarBridge::new_retirable("unused-sealed-pool", gate.clone());
    *bridge.inner.lock().await = fixture.inner.lock().await.take();
    let boundary = bridge.retirement.as_ref().unwrap();
    boundary.begin_start().unwrap();
    boundary.finish_start().unwrap();
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    let _closed = control.seal_admission(&proof).await.unwrap();
    let mut results = Vec::new();
    for cmd in ["browser_status", "close_idle", "register_proxy"] {
        results.push(bridge.call_with_policy(cmd, json!({}), Duration::from_secs(1), false).await);
    }
    let unexpected_frame = frames.try_recv().is_ok();
    let mut inner = bridge.inner.lock().await;
    let child = &mut inner.as_mut().unwrap().child;
    child.kill().await.unwrap();
    child.wait().await.unwrap();
    drop(inner);
    tokio::time::timeout(Duration::from_secs(2), reader).await.unwrap().unwrap();
    assert!(!unexpected_frame);
    assert!(results.iter().all(|r| r.as_ref().is_err_and(|e| e.contains("Closed"))));
}

#[tokio::test]
async fn incomplete_startup_cannot_claim_absence_or_commit_proxy_registration() {
    let (gate, _host) = OperationGate::new_with_legacy_host(8).unwrap();
    let (bridge, _control) = SidecarBridge::new_retirable("unused-incomplete-pool", gate);
    let account = AccountId::new();
    let absent = bridge.inspect_browser(&account).await.unwrap();
    bridge.retirement.as_ref().unwrap().begin_start().unwrap();
    let status = bridge.inspect_browser(&account).await.unwrap_err();
    let close = bridge.close_observed_browser(&account, &absent).await.unwrap_err();
    let unchanged = bridge.register_proxy(&account, None).await.unwrap_err();
    let changed = bridge.register_proxy(&account, Some("http://proxy.invalid:8080".into()))
        .await.unwrap_err();
    for error in [status, close, unchanged, changed] {
        assert!(error.contains("ProcessUnconfirmed"));
    }
    let cell = bridge.route_state(&account).await;
    let route = cell.lock().await;
    assert_eq!(route.current.revision, 0);
    assert!(route.current.proxy.is_none());
    assert_eq!(route.pending.as_ref().unwrap().revision, 1);
    drop(route);
    assert!(bridge.route_snapshot(&account).await.is_err());
    assert!(bridge.inner.lock().await.is_none());
}

#[tokio::test]
async fn existing_host_policy_writer_is_registered_for_retirement() {
    let (mut bridge, mut frames, reader) = super::proxy_tests::echo_bridge(None).await;
    let owned = tasks::TransportTasks::new();
    bridge.transport_tasks = Some(owned.clone());
    let result = bridge.call_with_policy("fixture", json!({}), Duration::from_secs(2), false).await;
    owned.seal().await;
    let joined = tokio::time::timeout(Duration::from_secs(2), owned.join()).await;
    let frame = frames.try_recv();
    super::proxy_tests::finish(bridge, reader).await;
    assert!(result.is_ok());
    assert_eq!(joined.unwrap().unwrap(), 1);
    assert_eq!(frame.unwrap()["cmd"], "fixture");
}

#[tokio::test]
async fn observed_mode_rejects_pool_and_route_work_before_startup() {
    let mut bridge = SidecarBridge::new("unused-observed-pool");
    bridge.mode = observer::ProcessMode::Observed {
        script: "unused-observed-script".into(), profile_nonce: uuid::Uuid::new_v4(),
    };
    let account = AccountId::new();
    assert!(bridge.call_account(&account, "mint", json!({})).await.is_err());
    assert!(bridge.route_for_command(&account).await.is_err());
    assert!(bridge.register_proxy(&account, Some("http://proxy.invalid:8080".into()))
        .await.is_err());
    assert!(bridge.inspect_browser(&account).await.is_err());
    for cmd in ["browser_status", "close_idle", "register_proxy"] {
        assert!(bridge.call_with_policy(cmd, json!({}), Duration::from_secs(1), false)
            .await.unwrap_err().contains("account-bound scope"));
    }
    assert!(bridge.inner.lock().await.is_none());
    assert!(bridge.routes.lock().await.is_empty());
}

#[tokio::test]
async fn sealed_route_paths_cannot_launch_or_change_local_proxy_state() {
    let (gate, _host) = OperationGate::new_with_legacy_host(8).unwrap();
    let (bridge, control) = SidecarBridge::new_retirable("unused-pool-route", gate.clone());
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    let _closed = control.seal_admission(&proof).await.unwrap();
    let account = AccountId::new();
    assert!(bridge.route_for_command(&account).await.is_err());
    assert!(bridge.register_proxy(&account, Some("http://proxy.invalid:8080".into()))
        .await.unwrap_err().contains("Closed"));
    assert!(bridge.inspect_browser(&account).await.unwrap_err().contains("Closed"));
    assert!(bridge.inner.lock().await.is_none());
    assert!(bridge.routes.lock().await.is_empty());
}

#[tokio::test]
async fn sealed_policy_entry_rejects_every_pool_io_command() {
    let (gate, _host) = OperationGate::new_with_legacy_host(8).unwrap();
    let (bridge, control) = SidecarBridge::new_retirable("unused-pool-merge", gate.clone());
    let mut ticket = gate.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    let _closed = control.seal_admission(&proof).await.unwrap();
    for cmd in ["browser_status", "close_idle", "register_proxy"] {
        let error = bridge.call_with_policy(cmd, json!({}), Duration::from_secs(1), false)
            .await.unwrap_err();
        assert!(error.contains("Closed"), "{cmd}: {error}");
    }
    assert!(bridge.inner.lock().await.is_none());
    assert!(bridge.routes.lock().await.is_empty());
}
