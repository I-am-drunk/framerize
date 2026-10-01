//! Smoke test: launch real Chromium, attach CDP, mint a Castle-shaped value on
//! framer.com. Run: cargo test -p framer-host -- --nocapture (needs chromium).
use framer_host::CdpBridge;
use framer_core::ids::AccountId;
use framer_core::port::BrowserBridge;

#[ignore] // needs a real Chrome binary; run: cargo test -p framer-host -- --ignored
#[tokio::test]
async fn launches_and_mints_castle_token() {
    let dir = std::env::temp_dir().join("framerize-host-test");
    let bridge = CdpBridge::new("chromium-browser", &dir);
    let account = AccountId::new();
    let token = bridge.castle_token(&account).await.expect("mint should succeed");
    assert!(token.0.len() > 1000, "castle tokens are ~5.7KB, got {}", token.0.len());
    assert!(token.0.contains('|'), "token has the <id>|<payload> shape");
    // device record persisted
    assert!(dir.join(account.0.to_string()).join("framerize-device.json").exists());
    // second mint: fresh token, same device
    let token2 = bridge.castle_token(&account).await.expect("second mint");
    assert_ne!(token.0, token2.0);
    let head = |t: &str| t.split('|').next().unwrap_or("").to_string();
    assert_eq!(head(&token.0), head(&token2.0), "device prefix stable across mints");
}
