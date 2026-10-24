//! Inert metadata only. No SDK, child process, account or token read.
use super::*;
use serde_json::json;

fn report() -> Value {
    json!({"version":1,"sdk":"framer-embedded-v2","selection":"configured-api",
        "status":"passed","failure_phase":null,"device_failure":null,
        "configure_calls":1,"token_method_calls":1,"device_storage_seeded":true,
        "device_storage_unchanged":true,"window_closed":true,"sources_unchanged":true,
        "async_errors":0,"diagnostics":2,"late_settlements":0,
        "method_ms":300.0,"elapsed_ms":350.0})
}
#[test]
fn completed_report_returns_only_timing_and_diagnostic_count() {
    assert_eq!(decode(&report(), 1000), Some(V2OperationMetadata {
        method_ms: 300.0, elapsed_ms: 350.0, diagnostics: 2,
    }));
}

#[test]
fn failed_or_unknown_selection_cannot_be_completed() {
    for (key, bad) in [("version", json!(2)), ("sdk", json!("v3")),
        ("selection", json!("wrapper")), ("status", json!("failed")),
        ("failure_phase", json!("mint")), ("device_failure", json!("local_value"))] {
        let mut value = report();
        value[key] = bad;
        assert_eq!(decode(&value, 1000), None, "{key}");
    }
}

#[test]
fn missing_fields_and_raw_handoff_or_account_claims_are_rejected() {
    for key in FIELDS {
        let mut value = report();
        value.as_object_mut().unwrap().remove(*key);
        assert!(decode(&value, 1000).is_none(), "{key}");
    }
    for key in ["token", "token_handoff", "account", "cookie", "message"] {
        let mut value = report();
        value[key] = json!("private-fixture-marker");
        assert!(decode(&value, 1000).is_none(), "{key}");
    }
}

#[test]
fn identity_and_cleanup_flags_require_actual_true_booleans() {
    for key in ["device_storage_seeded", "device_storage_unchanged", "window_closed",
        "sources_unchanged"] {
        for bad in [json!(false), json!(1), json!("true"), Value::Null] {
            let mut value = report();
            value[key] = bad;
            assert!(decode(&value, 1000).is_none(), "{key}");
        }
    }
}

#[test]
fn method_counts_fault_counts_and_integer_types_are_checked() {
    for (key, bad) in [("configure_calls", json!(0)), ("configure_calls", json!(2)),
        ("token_method_calls", json!(0)), ("token_method_calls", json!(2)),
        ("async_errors", json!(1)), ("late_settlements", json!(1)),
        ("version", json!(true)), ("version", json!(1.0)),
        ("diagnostics", json!(1_000_001)), ("diagnostics", json!(-1)),
        ("configure_calls", json!(true)), ("token_method_calls", json!(1.0))] {
        let mut value = report();
        value[key] = bad;
        assert!(decode(&value, 1000).is_none(), "{key}");
    }
}

#[test]
fn method_and_total_time_must_fit_the_supplied_budget() {
    for (key, bad) in [("method_ms", json!(-1)), ("method_ms", json!(5000.1)),
        ("method_ms", json!(false)), ("elapsed_ms", json!(-1)),
        ("elapsed_ms", json!(299)), ("elapsed_ms", json!(1000.1)),
        ("elapsed_ms", json!("NaN"))] {
        let mut value = report();
        value[key] = bad;
        assert!(decode(&value, 1000).is_none(), "{key}");
    }
    for budget in [0, 349, 16001] { assert!(decode(&report(), budget).is_none()); }
}

#[test]
fn component_numeric_boundaries_remain_accepted() {
    let mut value = report();
    value["method_ms"] = json!(5000);
    value["elapsed_ms"] = json!(16000);
    value["diagnostics"] = json!(1_000_000);
    assert!(decode(&value, 16000).is_some());
    value["method_ms"] = json!(0);
    value["elapsed_ms"] = json!(0);
    assert!(decode(&value, 1).is_some());
}

#[test]
fn v2_completion_cannot_satisfy_the_existing_v3_validator() {
    let bytes = serde_json::to_vec(&report()).unwrap();
    assert!(completed(&bytes, 1000).is_some());
    assert!(crate::metadata::accept_mode(&bytes, 1000, false).is_none());
    assert!(crate::metadata::accept_mode(&bytes, 1000, true).is_none());
}

#[test]
fn malformed_nonfinite_or_oversized_input_has_no_completion() {
    for bytes in [b"".as_slice(), b"{", b"[]", b"null", b"{} {}",
        b"{\"method_ms\":NaN}", b"{\"method_ms\":1e999}"] {
        assert!(completed(bytes, 1000).is_none());
    }
    let mut bytes = serde_json::to_vec(&report()).unwrap();
    bytes.resize(16385, b' ');
    assert!(completed(&bytes, 1000).is_none());
}

#[test]
fn duplicate_failure_keys_cannot_mask_failure() {
    let mut value = report();
    value["device_failure"] = json!("local_value");
    for suffix in [br#","device_failure":null}"#.as_slice(),
        br#","device_\u0066ailure":null}"#] {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(bytes.pop(), Some(b'}'));
        bytes.extend_from_slice(suffix);
        let decoded: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(decoded["device_failure"].is_null());
        assert!(completed(&bytes, 1000).is_none());
    }
}

fn decode(value: &Value, budget: u64) -> Option<V2OperationMetadata> {
    completed(&serde_json::to_vec(value).unwrap(), budget)
}
