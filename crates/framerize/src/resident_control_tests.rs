use super::*;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::Semaphore;

fn fixture() -> (wire::Binding, CapturedPage, Value) {
    let binding = wire::Binding { account: framer_core::ids::AccountId(uuid::Uuid::from_u128(1)),
        application: 17, host: "fixture-host".into(), incarnation: 3,
        lease: uuid::Uuid::from_u128(2) };
    let page = CapturedPage { application_generation: binding.application,
        host_generation: binding.host.clone(), account_incarnation: binding.incarnation,
        page_url: "https://framer.com/login".into(), device_id: "fixture-device".into(),
        effective_proxy: None, wrapper_url: "https://app.framerstatic.com/chunk-AAAAAAAA.mjs".into(),
        wrapper_export: "b".into() };
    let value = json!({"version": 1, "account": binding.account.0,
        "application_generation": binding.application, "host_generation": binding.host,
        "account_incarnation": binding.incarnation, "lease": binding.lease,
        "state": "acquired", "browser_settled": true, "token": "fixture-token",
        "cookie_header": "session=fixture; __cuid=fixture-device",
        "page_url": page.page_url, "device": page.device_id, "effective_proxy": null});
    (binding, page, value)
}

#[test]
fn acquire_checks_actual_page_without_inventing_editor_url() {
    let (binding, page, value) = fixture();
    let acquired = wire::acquired(&binding, &page, value).unwrap();
    assert_eq!(acquired.token.0, "fixture-token");
    assert_eq!(acquired.header, "session=fixture; __cuid=fixture-device");
}

#[test]
fn acquire_rejects_identity_route_page_and_type_drift() {
    for (key, replacement) in [
        ("version", json!(2)), ("account", json!(uuid::Uuid::from_u128(3))),
        ("application_generation", json!(18)), ("host_generation", json!("other-host")),
        ("account_incarnation", json!(4)), ("lease", json!(uuid::Uuid::from_u128(4))),
        ("page_url", json!("https://framer.com/projects/guessed")),
        ("device", json!("other-device")), ("effective_proxy", json!("http://proxy.invalid")),
        ("browser_settled", json!(1)), ("state", json!("released")),
        ("token", json!("bad\nvalue")), ("cookie_header", json!("")),
    ] {
        let (binding, page, mut value) = fixture();
        value[key] = replacement;
        assert!(wire::acquired(&binding, &page, value).is_err(), "field {key}");
    }
}

#[test]
fn acquire_rejects_extra_and_missing_fields() {
    let (binding, page, mut extra) = fixture();
    extra["unexpected"] = json!("private");
    assert!(wire::acquired(&binding, &page, extra).is_err());
    let (_, _, mut missing) = fixture();
    missing.as_object_mut().unwrap().remove("effective_proxy");
    assert!(wire::acquired(&binding, &page, missing).is_err());
}

#[test]
fn finish_requires_exact_fenced_browser_settlement() {
    let (binding, _, mut receipt) = fixture();
    for key in ["token", "cookie_header", "page_url", "device", "effective_proxy"] {
        receipt.as_object_mut().unwrap().remove(key);
    }
    receipt["state"] = json!("fenced");
    assert!(wire::settled(&binding, receipt.clone()));
    for (key, bad) in [("browser_settled", json!(false)), ("state", json!("released")),
        ("application_generation", json!(18)), ("lease", json!(uuid::Uuid::from_u128(8)))] {
        let mut bad_receipt = receipt.clone();
        bad_receipt[key] = bad;
        assert!(!wire::settled(&binding, bad_receipt));
    }
    receipt["token"] = json!("unexpected-private-material");
    assert!(!wire::settled(&binding, receipt));
}

#[test]
fn baseline_requires_same_lease_bearer_jar_and_page() {
    let (binding, page, _) = fixture();
    let identity = Identity { account: binding.account,
        user: framer_core::ids::FramerUserId("fixture-user".into()),
        team: framer_core::ids::TeamId("fixture-team".into()),
        project: framer_core::ids::ProjectId("fixture-project".into()),
        chat_session: "fixture-conversation".into() };
    let path = "/ai/v3/responses/";
    let mut receipt = BaselineReceipt { accepted: true, status: Some(200),
        identity: identity.clone(), configured_proxy_revision: 0, path: path.into(),
        application_generation: binding.application, host_generation: binding.host.clone(),
        account_incarnation: binding.incarnation, lease: binding.lease,
        page_url: page.page_url.clone(), device_id: page.device_id.clone(),
        cookie_header: "session=fixture".into(), effective_proxy: None };
    assert!(wire::baseline_matches(&binding, &page, &identity, path, "session=fixture", &receipt));
    receipt.path = "/ai/v3/messages/".into();
    assert!(!wire::baseline_matches(&binding, &page, &identity, path, "session=fixture", &receipt));
    receipt.path = path.into();
    receipt.cookie_header.push_str("; added=value");
    assert!(!wire::baseline_matches(&binding, &page, &identity, path, "session=fixture", &receipt));
}

async fn retained_cancel(drop_waiter: bool, expired: bool) {
    let capacity = Arc::new(Semaphore::new(1));
    let permit = capacity.clone().acquire_owned().await.unwrap();
    let stop = Arc::new(Stop(watch::channel(false).0));
    let guard = CancelOnDrop(stop.clone());
    let (finish, actual) = oneshot::channel::<()>();
    let (noticed, notice) = oneshot::channel::<()>();
    let worker_stop = stop.clone();
    let deadline = if expired { Instant::now() }
        else { Instant::now() + Duration::from_secs(30) };
    let work = async move { let _permit = permit; actual.await.unwrap(); };
    let worker = tokio::spawn(async move {
        retain_until_settled(work, &worker_stop, deadline, || {
            let _ = noticed.send(());
        }).await;
    });
    if drop_waiter { drop(guard); } else if !expired { stop.cancel(); }
    tokio::time::timeout(Duration::from_secs(2), notice).await.unwrap().unwrap();
    assert!(!worker.is_finished());
    assert_eq!(capacity.available_permits(), 0);
    finish.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), worker).await.unwrap().unwrap();
    assert_eq!(capacity.available_permits(), 1);
}

#[tokio::test]
async fn cancellation_retains_capacity_until_actual_settlement() { retained_cancel(false, false).await; }
#[tokio::test]
async fn dropped_waiter_retains_capacity_until_actual_settlement() { retained_cancel(true, false).await; }
#[tokio::test]
async fn deadline_retains_capacity_until_actual_settlement() { retained_cancel(false, true).await; }

#[tokio::test]
async fn captured_fallback_proxy_is_distinct_from_logical_configuration() {
    let native = NativeAccountClient::new().unwrap();
    let effective = Some("http://127.0.0.1:1".to_owned());
    let scope = native.open_account(framer_core::ids::AccountId(uuid::Uuid::from_u128(5)),
        framer_core::port::Session {
            active_user: framer_core::ids::FramerUserId("fixture-user".into()),
            cookie_header: "session=fixture".into(),
        }, NativeHttpLimits::default(), owner::egress(&effective)).unwrap();
    // Client construction is inert; this fixture makes no HTTP request.
    assert!(owner::routes_match(&scope, &None, &None, &effective));
    assert!(!owner::routes_match(&scope, &effective, &None, &effective));
    assert!(!owner::routes_match(&scope, &None, &None, &None));
    assert!(native.close_account(&scope).unwrap().await.is_ok());
}
