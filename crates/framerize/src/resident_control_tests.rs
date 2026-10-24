use super::*;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::Semaphore;

const HEADER: &str = "session=fixture; __cuid=fixture-device";
const SDK_DEVICE: &str = "0123456789ab4def8123456789abcdef";

#[test]
fn sdk_context_requires_exact_owned_snapshot_and_no_token() {
    for state in ["sdk_admitted", "sdk_complete"] {
        let (binding, page, mut value) = fixture();
        value["state"] = json!(state);
        value["sdk_device"] = json!(SDK_DEVICE);
        let device = wire::sdk_context(&binding, &page, HEADER, value.clone(), state).unwrap();
        assert_eq!(device.value(), SDK_DEVICE);
        assert_ne!(device.value(), page.device_id);
        for (key, change) in [("cookie_header", json!("different")),
            ("browser_settled", json!(false)), ("provenance", json!("different")),
            ("page_url", json!("https://framer.com/login?different")),
            ("effective_proxy", json!("http://other.invalid")),
            ("sdk_device", Value::Null), ("sdk_device", json!("fixture-device")),
            ("sdk_device", json!("01234567-89ab-4def-8123-456789abcdef")),
            ("sdk_device", json!("0123456789ab6def8123456789abcdef")),
            ("sdk_device", json!("0123456789ab4def7123456789abcdef")),
            ("sdk_device", json!("0123456789AB4def8123456789abcdef")),
            ("token", json!("unexpected-token")), ("state", json!("minted"))] {
            let mut changed = value.clone();
            changed[key] = change;
            assert!(wire::sdk_context(&binding, &page, HEADER, changed, state).is_err());
        }
    }
}

fn baseline_fixture() -> (wire::Binding, CapturedPage, Value) {
    let (binding, page, mut value) = fixture();
    value["state"] = json!("baseline_complete");
    value["status"] = json!(200);
    value["body_complete"] = json!(true);
    value["json_object"] = json!(true);
    value["accepted"] = json!(true);
    value["refusal"] = Value::Null;
    (binding, page, value)
}

fn minted_fixture() -> (wire::Binding, CapturedPage, Value) {
    let (binding, page, mut value) = fixture();
    value["state"] = json!("minted");
    value["token"] = json!("fixture-token");
    (binding, page, value)
}

fn fixture() -> (wire::Binding, CapturedPage, Value) {
    let binding = wire::Binding { account: framer_core::ids::AccountId(uuid::Uuid::from_u128(1)),
        application: 17, host: "fixture-host".into(), incarnation: 3,
        lease: uuid::Uuid::from_u128(2) };
    let page = CapturedPage { application_generation: binding.application,
        sidecar_incarnation: uuid::Uuid::from_u128(6),
        provenance: uuid::Uuid::from_u128(7).to_string(),
        host_generation: binding.host.clone(), account_incarnation: binding.incarnation,
        page_url: "https://framer.com/login".into(), device_id: "fixture-device".into(),
        effective_proxy: None, wrapper_url: "https://app.framerstatic.com/chunk-AAAAAAAA.mjs".into(),
        wrapper_export: "b".into() };
    let value = json!({"version": 2, "account": binding.account.0,
        "application_generation": binding.application, "host_generation": binding.host,
        "account_incarnation": binding.incarnation, "lease": binding.lease,
        "state": "acquired", "browser_settled": true, "provenance": page.provenance,
        "cookie_header": "session=fixture; __cuid=fixture-device",
        "page_url": page.page_url, "device": page.device_id, "effective_proxy": null});
    (binding, page, value)
}

#[test]
fn acquire_checks_actual_page_without_inventing_editor_url() {
    let (binding, page, value) = fixture();
    let acquired = wire::acquired(&binding, &page, value).unwrap();
    assert_eq!(acquired.header, "session=fixture; __cuid=fixture-device");
}

#[test]
fn acquire_rejects_identity_route_page_and_type_drift() {
    for (key, replacement) in [
        ("version", json!(1)), ("account", json!(uuid::Uuid::from_u128(3))),
        ("application_generation", json!(18)), ("host_generation", json!("other-host")),
        ("account_incarnation", json!(4)), ("lease", json!(uuid::Uuid::from_u128(4))),
        ("page_url", json!("https://framer.com/projects/guessed")),
        ("device", json!("other-device")), ("effective_proxy", json!("http://proxy.invalid")),
        ("browser_settled", json!(1)), ("state", json!("released")),
        ("provenance", json!(uuid::Uuid::from_u128(8))), ("cookie_header", json!("")),
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
    for key in ["provenance", "cookie_header", "page_url", "device", "effective_proxy"] {
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
fn baseline_acceptance_requires_the_complete_successful_response() {
    let (binding, page, value) = baseline_fixture();
    let baseline = wire::baseline(&binding, &page, HEADER, value).unwrap();
    assert!(baseline.permits_mint());
    assert!(baseline.refusal.is_none());
    assert_eq!(baseline.header, HEADER);
}

#[test]
fn v2_acquire_rejects_tokens_and_missing_or_invalid_provenance() {
    for token in [json!("fixture-token"), Value::Null] {
        let (binding, page, mut value) = fixture();
        value["token"] = token;
        assert!(wire::acquired(&binding, &page, value).is_err());
    }
    let (binding, mut page, mut value) = fixture();
    value.as_object_mut().unwrap().remove("provenance");
    assert!(wire::acquired(&binding, &page, value).is_err());
    let (_, _, mut value) = fixture();
    page.provenance = uuid::Uuid::nil().to_string();
    value["provenance"] = json!(page.provenance);
    assert!(wire::acquired(&binding, &page, value).is_err());
}

#[test]
fn baseline_and_mint_reject_context_drift_even_with_success_flags() {
    for (key, bad) in [
        ("version", json!(1)), ("account", json!(uuid::Uuid::from_u128(8))),
        ("application_generation", json!(18)), ("host_generation", json!("other")),
        ("account_incarnation", json!(4)), ("lease", json!(uuid::Uuid::from_u128(8))),
        ("provenance", json!(uuid::Uuid::from_u128(8))),
        ("page_url", json!("https://framer.com/projects/guessed")),
        ("device", json!("other")), ("effective_proxy", json!("http://proxy.invalid")),
        ("cookie_header", json!("session=fixture; __cuid=fixture-device; drift=1")),
    ] {
        let (binding, page, mut baseline) = baseline_fixture();
        baseline[key] = bad.clone();
        assert!(wire::baseline(&binding, &page, HEADER, baseline).is_err(), "{key}");
        let (_, _, mut minted) = minted_fixture();
        minted[key] = bad;
        assert!(wire::minted(&binding, &page, HEADER, minted).is_err(), "{key}");
    }
}

#[test]
fn baseline_failure_or_inconsistent_success_never_permits_mint() {
    for (key, bad) in [("status", json!(403)), ("status", Value::Null),
        ("body_complete", json!(false)), ("json_object", json!(false)),
        ("browser_settled", json!(false))] {
        let (binding, page, mut value) = baseline_fixture();
        value[key] = bad;
        assert!(wire::baseline(&binding, &page, HEADER, value.clone()).is_err());
        value["accepted"] = json!(false);
        let failed = wire::baseline(&binding, &page, HEADER, value).unwrap();
        assert!(!failed.permits_mint());
    }
    let (binding, page, mut value) = baseline_fixture();
    value["accepted"] = json!(false);
    assert!(!wire::baseline(&binding, &page, HEADER, value).unwrap().permits_mint());
    use framer_api::classify::{Refusal, RefusalBasis};
    for (policy, basis, expected_policy, expected_basis) in [
        ("castle", "framer_security_envelope", Refusal::Castle, RefusalBasis::FramerSecurityEnvelope),
        ("castle", "framer_marker", Refusal::Castle, RefusalBasis::FramerMarker),
        ("upstream", "provider_marker", Refusal::Upstream, RefusalBasis::ProviderMarker),
        ("castle", "elapsed_fallback", Refusal::Castle, RefusalBasis::ElapsedFallback),
        ("upstream", "elapsed_fallback", Refusal::Upstream, RefusalBasis::ElapsedFallback),
    ] {
        for settled in [true, false] {
            let (binding, page, mut value) = baseline_fixture();
            value["status"] = json!(403);
            value["accepted"] = json!(false);
            value["browser_settled"] = json!(settled);
            value["json_object"] = json!(false);
            value["refusal"] = json!({"policy":policy, "basis":basis});
            let baseline = wire::baseline(&binding, &page, HEADER, value).unwrap();
            let mut report = Report::default();
            assert_eq!(attempt::record_baseline(&mut report, &baseline), Err(Failure::Baseline));
            let refusal = report.refusal.unwrap();
            assert_eq!(refusal.policy, expected_policy);
            assert_eq!(refusal.basis, expected_basis);
            assert_eq!(report.baseline_status, Some(403));
            assert_eq!(report.cleanup_confirmed, settled);
            assert!(report.baseline_body_complete);
            assert!(!report.baseline_accepted && !report.native_invoked);
        }
    }
}

#[test]
fn baseline_rejects_non_boolean_flags_and_non_http_status() {
    for (key, bad) in [("status", json!(200.0)), ("status", json!(true)),
        ("status", json!(-1)), ("status", json!(600)), ("status", json!("200")),
        ("accepted", json!(1)), ("body_complete", json!(1)),
        ("json_object", json!(1)), ("browser_settled", json!(1))] {
        let (binding, page, mut value) = baseline_fixture();
        value[key] = bad;
        assert!(wire::baseline(&binding, &page, HEADER, value).is_err(), "{key}");
    }
    for bad in [json!(true), json!("castle"), json!([]), json!({}),
        json!({"policy":"castle"}), json!({"basis":"framer_marker"}),
        json!({"policy":"other", "basis":"framer_marker"}),
        json!({"policy":"castle", "basis":"other"}),
        json!({"policy":"castle", "basis":"provider_marker"}),
        json!({"policy":"upstream", "basis":"framer_marker"}),
        json!({"policy":"upstream", "basis":"framer_security_envelope"}),
        json!({"policy":"castle", "basis":"framer_marker", "body":"private"})] {
        let (binding, page, mut value) = baseline_fixture();
        value["status"] = json!(403);
        value["accepted"] = json!(false);
        value["refusal"] = bad;
        assert!(wire::baseline(&binding, &page, HEADER, value).is_err());
    }
    for (status, complete) in [(200, true), (500, true), (403, false)] {
        let (binding, page, mut value) = baseline_fixture();
        value["status"] = json!(status);
        value["accepted"] = json!(false);
        value["body_complete"] = json!(complete);
        value["refusal"] = json!({"policy":"castle", "basis":"framer_marker"});
        assert!(wire::baseline(&binding, &page, HEADER, value).is_err());
    }
}

#[test]
fn baseline_and_mint_require_exact_schema_and_current_token_context() {
    let (binding, page, baseline) = baseline_fixture();
    for key in baseline.as_object().unwrap().keys() {
        let mut missing = baseline.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(wire::baseline(&binding, &page, HEADER, missing).is_err());
    }
    let (_, _, minted) = minted_fixture();
    let result = wire::minted(&binding, &page, HEADER, minted.clone()).unwrap();
    assert_eq!(result.token.0, "fixture-token");
    assert_eq!(result.header, HEADER);
    for key in minted.as_object().unwrap().keys() {
        let mut missing = minted.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(wire::minted(&binding, &page, HEADER, missing).is_err());
    }
    for bad in [json!(""), json!("bad\nvalue"), json!("t".repeat(16 * 1024 + 1))] {
        let mut value = minted.clone();
        value["token"] = bad;
        assert!(wire::minted(&binding, &page, HEADER, value).is_err());
    }
    let mut extra = baseline;
    extra["token"] = json!("unexpected");
    assert!(wire::baseline(&binding, &page, HEADER, extra).is_err());
    let mut extra = minted;
    extra["accepted"] = json!(true);
    assert!(wire::minted(&binding, &page, HEADER, extra).is_err());
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
async fn local_baseline_rejection_preserves_only_confirmed_native_cleanup() {
    use framer_host::sidecar::resident::ResidentError;
    for (error, previously_settled, expected) in [
        (ResidentError::InvalidBaseline, true, true),
        (ResidentError::InvalidBaseline, false, false),
        (ResidentError::Unconfirmed, true, false),
        (ResidentError::OwnerUnavailable, true, false),
        (ResidentError::Deadline, true, false),
        (ResidentError::OutOfOrder, true, false),
    ] {
        let native = NativeAccountClient::new().unwrap();
        let scope = native.open_account(framer_core::ids::AccountId::new(),
            framer_core::port::Session {
                active_user: framer_core::ids::FramerUserId("fixture-user".into()),
                cookie_header: "session=fixture".into(),
            }, NativeHttpLimits::default(), owner::egress(&None)).unwrap();
        let pending = native.prepare_owned::<(), _, _>(&scope,
            Instant::now() + Duration::from_secs(2), move |_, _| async move {
                PreparationResult {
                    value: Err(PreparationFailure::Rejected),
                    cleanup_confirmed: attempt::baseline_rejection_settled(error, previously_settled),
                }
            }).unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(2), pending).await.unwrap().is_err());
        let closed = native.close_account(&scope).unwrap();
        assert_eq!(tokio::time::timeout(Duration::from_secs(2), closed).await.unwrap().is_ok(),
            expected, "{error:?}, previously settled: {previously_settled}");
    }
}

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
