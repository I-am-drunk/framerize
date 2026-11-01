use super::*;

#[tokio::test]
async fn absent_status_and_close_never_start_a_host() {
    let account = AccountId::new();
    let root = std::env::temp_dir().join(format!("uncreated-lifecycle-{}", account.0));
    let bridge = SidecarBridge::new(&root);
    let status = bridge.browser_status(&account).await.unwrap();
    assert_eq!(status.state, BrowserAccountState::Absent);
    assert_eq!(status.host_generation, None);
    let closed = bridge.close_idle_browser(&account, &status).await.unwrap();
    assert!(closed.confirms_absence(&status));
    assert!(bridge.inner.lock().await.is_none());
    assert!(!root.exists(), "inspection created host state");
}

#[tokio::test]
async fn other_account_close_is_rejected_without_starting_a_host() {
    let account = AccountId::new();
    let bridge = SidecarBridge::new("unused-lifecycle-state");
    let other = absent_without_host(&AccountId::new());
    assert!(bridge.close_idle_browser(&account, &other).await.is_err());
    assert!(bridge.inner.lock().await.is_none());
}

#[tokio::test]
async fn existing_only_call_does_not_spawn_a_missing_host() {
    let bridge = SidecarBridge::new("unused-lifecycle-state");
    let result = bridge.call_with_policy("browser_status", json!({}), STATUS_TIMEOUT, false).await;
    assert_eq!(result.unwrap_err(), "sidecar unavailable: browser host has not been started");
    assert!(bridge.inner.lock().await.is_none());
}

#[tokio::test]
async fn existing_only_deadline_includes_waiting_for_the_host_lock() {
    let bridge = SidecarBridge::new("unused-lifecycle-state");
    let _locked = bridge.inner.lock().await;
    let result = tokio::time::timeout(Duration::from_secs(1),
        bridge.call_with_policy("browser_status", json!({}), Duration::from_millis(20), false))
        .await.unwrap();
    assert!(result.unwrap_err().starts_with("sidecar timeout after "));
}

#[test]
fn stale_or_active_close_receipts_do_not_release_capacity() {
    let mut expected = absent_without_host(&AccountId::new());
    expected.host_generation = Some("generation-a".into());
    expected.account_incarnation = 2;
    let mut result = IdleCloseResult { outcome: IdleCloseOutcome::Closed, status: expected.clone() };
    assert!(result.confirms_absence(&expected));
    result.status.account_incarnation += 1;
    assert!(!result.confirms_absence(&expected));
    result.status = expected.clone();
    result.status.host_generation = Some("generation-b".into());
    assert!(!result.confirms_absence(&expected));
    result.status = expected.clone();
    result.status.active_commands = 1;
    assert!(!result.confirms_absence(&expected));
    result.status = expected.clone();
    result.status.state = BrowserAccountState::Unconfirmed;
    assert!(!result.confirms_absence(&expected));
}

#[test]
fn remote_status_requires_an_account_and_host_identity() {
    let account = AccountId::new();
    let mut status = absent_without_host(&account);
    assert!(validate_status(&account, status.clone()).is_err());
    status.host_generation = Some("".into());
    assert!(validate_status(&account, status.clone()).is_err());
    status.host_generation = Some("generation".into());
    assert!(validate_status(&AccountId::new(), status.clone()).is_err());
    status.state = BrowserAccountState::Resident;
    assert!(validate_status(&account, status.clone()).is_err());
    status.account_incarnation = 1;
    assert_eq!(validate_status(&account, status.clone()).unwrap(), status);
}

#[test]
fn unknown_status_and_close_fields_are_rejected() {
    let account = AccountId::new();
    let mut status = serde_json::to_value(absent_without_host(&account)).unwrap();
    status["future_absence_claim"] = json!(true);
    assert!(serde_json::from_value::<BrowserAccountStatus>(status).is_err());
    let mut result = serde_json::to_value(IdleCloseResult {
        outcome: IdleCloseOutcome::Closed, status: absent_without_host(&account),
    }).unwrap();
    result["future_absence_claim"] = json!(true);
    assert!(serde_json::from_value::<IdleCloseResult>(result).is_err());
}

#[tokio::test]
async fn an_exited_host_is_uncertain_and_is_never_respawned_by_inspection() {
    let mut child = Command::new("python3").args(["-c", "pass"])
        .stdin(Stdio::piped()).stdout(Stdio::null()).kill_on_drop(true).spawn().unwrap();
    let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    tokio::time::timeout(Duration::from_secs(5), child.wait()).await.unwrap().unwrap();
    let pending = Arc::new(StdMutex::new(HashMap::new()));
    let (cancel, _rx) = mpsc::channel(1);
    let bridge = SidecarBridge::new("unused-lifecycle-state");
    *bridge.inner.lock().await = Some(Sidecar {
        child, stdin, pending: pending.clone(), cancel, next_id: Arc::new(AtomicU64::new(0)),
        healthy: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        failure: Arc::new(StdMutex::new(None)),
        incarnation: uuid::Uuid::new_v4(),
    });
    let error = bridge.browser_status(&AccountId::new()).await.unwrap_err();
    assert!(error.starts_with("host-lifecycle-unconfirmed:"));
    assert!(error.contains("exited with exit status: 0"), "{error}");
    let regular = bridge.call("fixture", json!({})).await.unwrap_err();
    assert!(regular.starts_with("host-lifecycle-unconfirmed:"));
    let mut locked = bridge.inner.lock().await;
    assert!(locked.as_mut().unwrap().child.try_wait().unwrap().is_some());
    assert!(pending.lock().unwrap().is_empty());
}
