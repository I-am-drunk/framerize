use super::*;

fn fixture(engine: EngineKind) -> (CdpBridge, PathBuf) {
    let root = std::env::temp_dir().join(format!("uncreated-proxy-auth-{}", uuid::Uuid::new_v4()));
    (CdpBridge::with_engine(root.join("no-browser"), &root, engine), root)
}

#[tokio::test]
async fn embedded_proxy_credentials_are_rejected_before_registration() {
    let proxies = [
        "http://alice:secret@proxy.invalid:8080",
        "https://alice@proxy.invalid:8443",
        "socks5://user%40team:p%3Ass@[2001:db8::4]:1080",
        "alice:secret@proxy.invalid:8080",
        "http=plain.invalid:80;https=https://alice:secret@proxy.invalid:443",
    ];
    for engine in [EngineKind::FullChrome, EngineKind::HeadlessShell] {
        let (bridge, root) = fixture(engine);
        for proxy in proxies {
            let error = bridge.register_account_proxy(&AccountId::new(), Some(proxy.into()))
                .await.unwrap_err();
            assert_eq!(error, "CDP browser proxy credentials are unsupported");
        }
        assert!(bridge.proxies.read().await.is_empty());
        assert!(bridge.routes.lock().await.is_empty());
        assert!(bridge.instances.lock().await.is_empty());
        assert!(!root.exists());
    }
}

#[tokio::test]
async fn rejected_credentials_preserve_existing_account_routes() {
    let (bridge, root) = fixture(EngineKind::FullChrome);
    let account = AccountId::new();
    let other = AccountId::new();
    bridge.register_account_proxy(&account, Some("http://saved.invalid:8080".into()))
        .await.unwrap();
    bridge.register_account_proxy(&other, Some("socks5://other.invalid:1080".into()))
        .await.unwrap();
    let before = bridge.proxies.read().await.clone();
    let error = bridge.set_proxy(&account, Some("http://secret:password@new.invalid".into()))
        .await.unwrap_err();
    assert_eq!(error, "CDP browser proxy credentials are unsupported");
    assert_eq!(*bridge.proxies.read().await, before);
    assert!(bridge.instances.lock().await.is_empty());
    assert!(!root.exists());
}

#[tokio::test]
async fn unauthenticated_proxy_forms_and_direct_routes_remain_supported() {
    let proxies = [
        "http://plain.invalid:8080", "https://plain.invalid:8443",
        "socks4://plain.invalid:1080", "socks5://plain.invalid:1080",
        "[2001:db8::4]:8080", "http://[2001:db8::4]:8080", "plain.invalid:8080",
        "http=one.invalid:80;https=two.invalid:443;socks=socks5://three.invalid:1080",
    ];
    for engine in [EngineKind::FullChrome, EngineKind::HeadlessShell] {
        let (bridge, root) = fixture(engine);
        let account = AccountId::new();
        for proxy in proxies {
            bridge.register_account_proxy(&account, Some(proxy.into())).await.unwrap();
            assert_eq!(bridge.proxies.read().await[&account.0.to_string()], proxy);
        }
        bridge.register_account_proxy(&account, None).await.unwrap();
        assert!(bridge.proxies.read().await.is_empty());
        assert!(bridge.instances.lock().await.is_empty());
        assert!(!root.exists());
    }
}
