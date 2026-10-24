//! Inert metadata only; no child process, SDK or runtime access.
use super::*;
use serde_json::json;

fn observed(value: &Value, code: Option<i32>) -> Failure {
    from_exit(code, &serde_json::to_vec(value).unwrap())
}

fn compact(stage: &str) -> Value {
    json!({"version":1, "status":"failed", "failure_phase":stage})
}

#[test]
fn compact_failure_retains_only_a_known_stage() {
    for label in ["input", "module-link", "mint", "handoff"] {
        let actual = observed(&compact(label), Some(2));
        assert_eq!(actual, Failure::WorkerStage(WorkerStage(label)));
    }
}

#[test]
fn full_failure_retains_stage_for_stateless_and_device_modes() {
    let mut value = full();
    assert_eq!(observed(&value, Some(2)), Failure::WorkerStage(WorkerStage("mint")));
    value["version"] = json!(2);
    value["device_storage_seeded"] = json!(true);
    value["device_storage_unchanged"] = json!(false);
    value["failure_phase"] = json!("device-storage");
    assert_eq!(observed(&value, Some(2)), Failure::WorkerStage(WorkerStage("device-storage")));
}

#[test]
fn unexpected_exit_or_passed_report_cannot_claim_a_failure_stage() {
    for code in [None, Some(0), Some(1), Some(3)] {
        assert_eq!(observed(&full(), code), Failure::Worker);
    }
    let mut value = full();
    value["status"] = json!("passed");
    assert_eq!(observed(&value, Some(2)), Failure::Worker);
}

#[test]
fn unknown_stage_or_extra_private_fields_stay_generic() {
    let mut extra = full();
    extra["message"] = json!("private-token-and-cookie-marker");
    for value in [compact("private-token-and-cookie-marker"), extra,
        json!({"version":1,"status":"failed","failure_phase":true})] {
        let result = observed(&value, Some(2));
        assert_eq!(result, Failure::Worker);
        assert!(!format!("{result:?}").contains("private"));
    }
}

#[test]
fn full_report_requires_exact_types_and_bounded_counters() {
    for (key, invalid) in [("version", json!(1.0)), ("wrapper_calls", json!(true)),
        ("diagnostics", json!(1_000_001)), ("method_ms", json!(-1)),
        ("v3_evaluated", json!(1)), ("token_handoff", json!(true)),
        ("branch", json!({"v3":0,"v2":0,"none":0,"errors":0,"extra":"private"}))] {
        let mut value = full();
        value[key] = invalid;
        assert_eq!(observed(&value, Some(2)), Failure::Worker, "{key}");
    }
}

#[test]
fn incomplete_and_wrong_version_schemas_are_not_diagnostics() {
    let original = full();
    for key in original.as_object().unwrap().keys() {
        let mut value = full();
        value.as_object_mut().unwrap().remove(key);
        assert_eq!(observed(&value, Some(2)), Failure::Worker, "{key}");
    }
    for mut value in [full(), compact("input")] {
        value["version"] = json!(2);
        assert_eq!(observed(&value, Some(2)), Failure::Worker);
    }
}

#[test]
fn malformed_or_oversized_metadata_stays_generic() {
    for bytes in [b"".as_slice(), b"{", b"[]", b"null", b"{\"version\":NaN}"] {
        assert_eq!(from_exit(Some(2), bytes), Failure::Worker);
    }
    let mut bytes = serde_json::to_vec(&compact("mint")).unwrap();
    bytes.resize(16385, b' ');
    assert_eq!(from_exit(Some(2), &bytes), Failure::Worker);
}

#[test]
fn annotated_failure_still_fails_the_success_validator() {
    let bytes = serde_json::to_vec(&full()).unwrap();
    assert_eq!(from_exit(Some(2), &bytes), Failure::WorkerStage(WorkerStage("mint")));
    assert!(crate::metadata::accept(&bytes, 16000).is_none());
}

fn full() -> Value {
    json!({"version":1,"status":"failed","failure_phase":"mint",
        "target_validated":true,"account_state_loaded":false,"wrapper_calls":1,
        "branch":{"v3":0,"v2":0,"none":1,"errors":1},"dynamic_imports":1,
        "v3_evaluated":true,"window_closed":true,"inspector_closed":true,
        "sources_unchanged":true,"async_errors":0,"diagnostics":1,
        "late_settlements":0,"method_ms":3.0,"elapsed_ms":5.0,
        "mint_to_handoff_ms":0.0,"token_handoff":false})
}
