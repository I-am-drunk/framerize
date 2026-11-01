use super::*;

#[tokio::test]
async fn cdp_registration_waits_only_for_its_own_launch_and_cancellation_does_not_mutate() {
    let (bridge, root) = fixture();
    let account = AccountId::new();
    let cell = bridge.route_cell(&account).await;
    let guard = cell.lock().await;
    let mut pending = Box::pin(bridge.register_account_proxy(
        &account, Some("http://saved.invalid:1234".into())));
    assert!(futures::poll!(pending.as_mut()).is_pending());
    bridge.register_account_proxy(&AccountId::new(), Some("http://other.invalid:4321".into()))
        .await.unwrap();
    drop(pending);
    drop(guard);
    assert!(!bridge.proxies.read().await.contains_key(&account.0.to_string()));
    assert!(!root.exists());
}

#[tokio::test]
async fn cdp_uncertain_start_refuses_implicit_relaunch_and_destructive_proxy_setter() {
    let (bridge, root) = fixture();
    let account = AccountId::new();
    bridge.route_cell(&account).await.lock().await.may_have_browser = true;
    let error = bridge.instance(&account).await.err().unwrap();
    assert_eq!(error, OWNED_STARTUP_UNCONFIRMED);
    assert!(bridge.set_proxy(&account, Some("http://new.invalid:4321".into())).await.is_err());
    assert!(bridge.rotate_identity(&account).await.is_err());
    assert!(bridge.proxies.read().await.is_empty());
    assert!(bridge.instances.lock().await.is_empty());
    assert!(!root.exists());
}

fn fixture() -> (CdpBridge, PathBuf) {
    let root = std::env::temp_dir().join(format!("uncreated-cdp-route-{}", uuid::Uuid::new_v4()));
    (CdpBridge::new(root.join("no-browser"), &root), root)
}

#[tokio::test]
async fn cdp_spawn_failure_without_a_child_allows_registration_retry() {
    let (bridge, root) = fixture();
    let account = AccountId::new();
    let result = bridge.instance(&account).await;
    assert!(result.err().unwrap().starts_with("launch chrome:"));
    bridge.register_account_proxy(&account, Some("http://saved.invalid:1234".into()))
        .await.unwrap();
    assert!(!bridge.route_cell(&account).await.lock().await.may_have_browser);
    assert!(bridge.instances.lock().await.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn cdp_bulk_registration_cannot_override_an_unconfirmed_accounts_route() {
    let (bridge, root) = fixture();
    let account = AccountId::new();
    bridge.register_account_proxy(&account, Some("http://saved.invalid:1234".into()))
        .await.unwrap();
    bridge.route_cell(&account).await.lock().await.may_have_browser = true;
    let changed = HashMap::from([(account.0.to_string(), "http://new.invalid:4321".into())]);
    assert!(bridge.set_proxies(changed).await.is_err());
    assert_eq!(bridge.proxies.read().await[&account.0.to_string()], "http://saved.invalid:1234");
    assert!(!root.exists());
}

#[tokio::test]
async fn cdp_invalid_proxy_is_rejected_without_registration_or_files() {
    let (bridge, root) = fixture();
    assert!(bridge.register_account_proxy(&AccountId::new(), Some(String::new())).await.is_err());
    assert!(bridge.proxies.read().await.is_empty());
    assert!(bridge.routes.lock().await.is_empty());
    assert!(!root.exists());
}

#[tokio::test]
async fn attached_browser_does_not_claim_to_apply_a_proxy() {
    let root = std::env::temp_dir().join(format!("uncreated-attach-{}", uuid::Uuid::new_v4()));
    let bridge = CdpBridge::with_engine("unused-address", &root, EngineKind::Attach);
    let account = AccountId::new();
    bridge.register_account_proxy(&account, None).await.unwrap();
    assert!(bridge.register_account_proxy(&account, Some("http://proxy.invalid:1234".into()))
        .await.is_err());
    assert!(bridge.instances.lock().await.is_empty());
    assert!(bridge.proxies.read().await.is_empty());
    assert!(!root.exists());
}

#[tokio::test]
async fn cdp_registration_is_nonlaunching_idempotent_and_can_clear_absent_route() {
    let (bridge, root) = fixture();
    let account = AccountId::new();
    let proxy = Some("http://saved.invalid:1234".into());
    bridge.register_account_proxy(&account, proxy.clone()).await.unwrap();
    bridge.register_account_proxy(&account, proxy).await.unwrap();
    assert_eq!(bridge.proxies.read().await.get(&account.0.to_string()).map(String::as_str),
        Some("http://saved.invalid:1234"));
    bridge.register_account_proxy(&account, None).await.unwrap();
    assert!(bridge.proxies.read().await.is_empty());
    assert!(bridge.instances.lock().await.is_empty());
    assert!(!root.exists());
}

#[tokio::test]
async fn cdp_started_or_unconfirmed_route_cannot_change_but_others_are_usable() {
    let (bridge, root) = fixture();
    let account = AccountId::new();
    let proxy = Some("http://saved.invalid:1234".into());
    bridge.register_account_proxy(&account, proxy.clone()).await.unwrap();
    bridge.route_cell(&account).await.lock().await.may_have_browser = true;
    bridge.register_account_proxy(&account, proxy).await.unwrap();
    assert!(bridge.register_account_proxy(&account, None).await.is_err());
    bridge.register_account_proxy(&AccountId::new(), Some("http://other.invalid:4321".into()))
        .await.unwrap();
    assert_eq!(bridge.proxies.read().await[&account.0.to_string()], "http://saved.invalid:1234");
    assert!(!root.exists());
}
