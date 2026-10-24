use super::*;

fn fixture() -> Value {
    json!({"version": 2, "account": Uuid::from_u128(1),
        "application_generation": 4, "bootstrap": Uuid::from_u128(2),
        "host_generation": "fixture-host", "account_incarnation": 3,
        "project": "fixture-project", "path": "/ai/v3/responses/",
        "proxy_revision": 7, "provenance": Uuid::from_u128(5),
        "page_url": "https://framer.com/login", "device": "fixture-device",
        "effective_proxy": null, "wrapper_export": "a",
        "wrapper_url": "https://app.framerstatic.com/chunk-ABCDEFG1.mjs",
        "browser_settled": true})
}

fn check(value: Value) -> Result<ResidentBootstrap, BootstrapError> {
    checked(value, AccountId(Uuid::from_u128(1)), ApplicationGeneration(4),
        Uuid::from_u128(6), Uuid::from_u128(2), 7, "fixture-project", "/ai/v3/responses/")
}

#[test]
fn login_page_and_launch_proxy_remain_actual_metadata() {
    let mut value = fixture();
    value["effective_proxy"] = json!("http://fixture.invalid:3128");
    let result = check(value).unwrap();
    assert_eq!(result.page_url, "https://framer.com/login");
    assert_eq!(result.effective_proxy.as_deref(), Some("http://fixture.invalid:3128"));
    assert_eq!(result.sidecar_incarnation, Uuid::from_u128(6));
    assert_eq!(result.proxy_revision, 7);
}

#[test]
fn identity_and_settlement_must_match_exactly() {
    for (key, replacement) in [
        ("version", json!(1)), ("account", json!(Uuid::from_u128(9))),
        ("application_generation", json!(5)), ("bootstrap", json!(Uuid::from_u128(9))),
        ("project", json!("another-project")), ("path", json!("/ai/v3/messages/")),
        ("proxy_revision", json!(8)), ("browser_settled", json!(false)),
        ("browser_settled", json!(1)), ("account_incarnation", json!(0)),
        ("provenance", json!(Uuid::nil())), ("device", json!("")),
        ("host_generation", json!("")), ("effective_proxy", json!("")),
    ] {
        let mut value = fixture();
        value[key] = replacement;
        assert!(matches!(check(value), Err(BootstrapError::Unconfirmed)), "{key}");
    }
}

#[test]
fn every_field_is_required_and_private_extras_are_rejected() {
    for key in fixture().as_object().unwrap().keys() {
        let mut value = fixture();
        value.as_object_mut().unwrap().remove(key);
        assert!(check(value).is_err(), "{key}");
    }
    for key in ["token", "cookie_header", "error", "extra"] {
        let mut value = fixture();
        value[key] = json!("synthetic-private");
        assert!(check(value).is_err(), "{key}");
    }
}

#[test]
fn page_and_cached_wrapper_urls_have_no_origin_fallback() {
    for url in ["https://framer.com.evil/login", "http://framer.com/login",
        "https://framer.com@evil/login", "https://framer.com:444/login",
        "https://framer.com\\@evil/login", "https://framer.com/\nlogin"] {
        let mut value = fixture(); value["page_url"] = json!(url);
        assert!(check(value).is_err());
    }
    for url in ["https://app.framerstatic.com/chunk-ABCDEFG1.mjs?other=1",
        "https://app.framerstatic.com/chunk-ABCDEFG1.mjs#copy",
        "https://evil/chunk-ABCDEFG1.mjs"] {
        let mut value = fixture(); value["wrapper_url"] = json!(url);
        assert!(check(value).is_err());
    }
    let mut value = fixture();
    value["wrapper_url"] = json!("https://app.framerstatic.com/chunk-ABCDEFG1.mjs?cb=123");
    assert!(check(value).is_ok());
}

#[tokio::test]
async fn unavailable_owner_never_starts_or_registers_a_host() {
    let bridge = Arc::new(SidecarBridge::new(std::path::PathBuf::from("/must-not-start")));
    assert!(matches!(bridge.resident_bootstrap(AccountId(Uuid::from_u128(1)),
        "fixture-project", "/ai/v3/responses/", &None).await,
        Err(BootstrapError::Unavailable)));
    assert!(bridge.inner.lock().await.is_none());
    assert!(bridge.routes.lock().await.is_empty());
}

fn unavailable() -> Value {
    json!({"version":2, "account":Uuid::from_u128(1), "application_generation":4,
        "bootstrap":Uuid::from_u128(2), "project":"fixture-project",
        "path":"/ai/v3/responses/", "proxy_revision":7, "status":"unavailable",
        "stage":"provenance", "browser_settled":true})
}

#[test]
fn only_exact_pre_acquire_refusal_confirms_no_work() {
    assert!(matches!(check(unavailable()), Err(BootstrapError::Unavailable)));
    for (key, value) in [
        ("version", json!(1)), ("account", json!(Uuid::from_u128(9))),
        ("application_generation", json!(5)), ("bootstrap", json!(Uuid::from_u128(9))),
        ("project", json!("other")), ("path", json!("/ai/v3/messages/")),
        ("proxy_revision", json!(8)), ("status", json!("failed")),
        ("stage", json!("internal")), ("stage", json!("timeout")),
        ("browser_settled", json!(false)), ("browser_settled", json!(1)),
    ] {
        let mut reply = unavailable(); reply[key] = value;
        assert!(matches!(check(reply), Err(BootstrapError::Unconfirmed)), "{key}");
    }
}

#[test]
fn no_work_receipt_requires_all_fields_and_rejects_extras() {
    for key in unavailable().as_object().unwrap().keys() {
        let mut reply = unavailable(); reply.as_object_mut().unwrap().remove(key);
        assert!(matches!(check(reply), Err(BootstrapError::Unconfirmed)), "{key}");
    }
    let mut reply = unavailable(); reply["token"] = json!("private-fixture");
    assert!(matches!(check(reply), Err(BootstrapError::Unconfirmed)));
}
