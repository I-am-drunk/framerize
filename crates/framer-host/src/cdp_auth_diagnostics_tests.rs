//! Inert authentication error-path regressions; no CDP connection is opened.
use super::*;

#[test]
fn auth_error_preview_preserves_utf8_at_the_byte_limit() {
    for (length, character) in [(399, "é"), (398, "中"), (397, "🦊")] {
        let prefix = "x".repeat(length);
        let body = format!("{prefix}{character}after");
        assert_eq!(body_preview(&json!({"body": body})), prefix);
    }
}

#[test]
fn auth_error_preview_keeps_short_and_exact_limit_bodies() {
    for body in [String::new(), "short é中🦊".into(), "é".repeat(200)] {
        assert_eq!(body_preview(&json!({"body": &body})), body);
    }
    assert_eq!(body_preview(&json!({"body": "x".repeat(401)})), "x".repeat(400));
}

#[test]
fn auth_error_preview_keeps_missing_or_non_string_body_empty() {
    for reply in [json!({}), json!({"body": null}), json!({"body": 7})] {
        assert_eq!(body_preview(&reply), "");
    }
}

#[tokio::test]
async fn malformed_session_is_redacted_before_browser_admission() {
    let secret = "synthetic-cdp-session-secret-without-equals";
    for engine in [EngineKind::FullChrome, EngineKind::HeadlessShell, EngineKind::Attach] {
        let root = std::env::temp_dir().join(format!("uncreated-cdp-auth-{}", uuid::Uuid::new_v4()));
        let bridge = CdpBridge::with_engine("unused-cdp-auth", &root, engine);
        let error = bridge.seed_session(&AccountId::new(), secret).await.unwrap_err();
        assert_eq!(error, "bad cookie header");
        assert!(!error.contains(secret));
        assert!(bridge.instances.lock().await.is_empty());
        assert!(bridge.routes.lock().await.is_empty());
        assert!(!root.exists());
    }
}
