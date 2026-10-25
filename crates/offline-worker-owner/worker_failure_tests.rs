//! Inert metadata and owner channels; no child process, SDK or service access.
use super::*;
use serde_json::json;
use futures::FutureExt;

const PHASE_LABELS: &[&str] = &["v3-module-load-error", "v3-module-load-timeout",
    "v3-configuration-error", "v3-configuration-timeout", "v3-token-generation-error",
    "v3-token-generation-timeout", "v3-sdk-error"];

fn observed(value: &Value, code: Option<i32>) -> Failure {
    from_exit(code, &serde_json::to_vec(value).unwrap())
}

fn completed_run(failure: Failure, cleanup: crate::Cleanup, cancelled: bool,
    expired: bool) -> crate::Run<()> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    assert!(sender.send(Err(failure)).is_ok());
    let (cancel, _receiver) = tokio::sync::watch::channel(cancelled);
    crate::Run {
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(
            if expired { 0 } else { 3600 }),
        pid: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
        result: Some(receiver),
        completion: crate::Completion(futures::future::ready(cleanup).boxed().shared()),
        cancel,
    }
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

#[test]
fn v3_phase_labels_require_full_failed_reports_in_both_modes() {
    assert_eq!(V3_STAGES, PHASE_LABELS);
    for &label in PHASE_LABELS {
        for version in [1, 2] {
            let mut value = full();
            value["version"] = json!(version);
            value["failure_phase"] = json!(label);
            if version == 2 {
                value["device_storage_seeded"] = json!(true);
                value["device_storage_unchanged"] = json!(true);
            }
            let bytes = serde_json::to_vec(&value).unwrap();
            assert_eq!(from_exit(Some(2), &bytes), Failure::WorkerStage(WorkerStage(label)));
            assert!(crate::metadata::accept(&bytes, 16000).is_none());
        }
    }
}

#[test]
fn v3_phase_labels_cannot_claim_compact_or_error_free_evidence() {
    for &label in PHASE_LABELS {
        assert_eq!(observed(&compact(label), Some(2)), Failure::Worker);
        let mut value = full();
        value["failure_phase"] = json!(label);
        value["branch"]["errors"] = json!(0);
        assert_eq!(observed(&value, Some(2)), Failure::Worker);
        value["branch"]["errors"] = json!(1);
        value["status"] = json!("passed");
        assert_eq!(observed(&value, Some(2)), Failure::Worker);
    }
}

#[test]
fn malformed_v3_phase_reports_and_unexpected_exits_stay_generic() {
    let mut original = full();
    original["failure_phase"] = json!("v3-configuration-error");
    for key in original.as_object().unwrap().keys() {
        let mut value = original.clone();
        value.as_object_mut().unwrap().remove(key);
        assert_eq!(observed(&value, Some(2)), Failure::Worker, "{key}");
    }
    for (key, invalid) in [("version", json!(1.0)), ("diagnostics", json!(1_000_001)),
        ("method_ms", json!(-1)), ("v3_evaluated", json!(1)),
        ("token_handoff", json!(true)), ("status", json!("passed")),
        ("branch", json!({"v3":0,"v2":0,"none":1,"errors":true}))] {
        let mut value = original.clone();
        value[key] = invalid;
        assert_eq!(observed(&value, Some(2)), Failure::Worker, "{key}");
    }
    for code in [None, Some(0), Some(1), Some(3)] {
        assert_eq!(observed(&original, code), Failure::Worker);
    }
}

#[test]
fn v3_phase_reports_never_surface_private_fields_or_unknown_labels() {
    for label in ["v3-sdk-timeout", "v3-module-load", "V3-configuration-error",
        "v3-token-generation-error ", "private-fixture-marker"] {
        let mut value = full(); value["failure_phase"] = json!(label);
        assert_eq!(observed(&value, Some(2)), Failure::Worker);
    }
    for key in ["message", "cause", "stack", "token", "castlePhase"] {
        let mut value = full(); value["failure_phase"] = json!("v3-sdk-error");
        value[key] = json!("private-fixture-marker");
        let actual = observed(&value, Some(2));
        assert_eq!(actual, Failure::Worker);
        assert!(!format!("{actual:?}").contains("private-fixture-marker"));
    }
}

#[test]
fn v3_phase_refinement_preserves_all_original_stage_schemas() {
    for &label in STAGES {
        let expected = Failure::WorkerStage(WorkerStage(label));
        assert_eq!(observed(&compact(label), Some(2)), expected);
        let mut value = full(); value["failure_phase"] = json!(label);
        assert_eq!(observed(&value, Some(2)), expected);
    }
}

#[tokio::test]
async fn v3_phase_keeps_owner_cancellation_and_deadline_precedence() {
    let mut value = full(); value["failure_phase"] = json!("v3-sdk-error");
    let failure = observed(&value, Some(2));
    for (cancelled, expired, expected) in [(true, false, Failure::Cancelled),
        (true, true, Failure::Cancelled), (false, true, Failure::Deadline)] {
        let mut run = completed_run(failure, crate::Cleanup::Unconfirmed, cancelled, expired);
        assert_eq!(run.result().await.err(), Some(expected));
        assert_eq!(run.completion().wait().await, crate::Cleanup::Unconfirmed);
    }
}

#[tokio::test]
async fn v3_phase_retains_the_original_cleanup_verdict() {
    let mut value = full(); value["failure_phase"] = json!("v3-sdk-error");
    let failure = observed(&value, Some(2));
    for cleanup in [crate::Cleanup::NotStarted, crate::Cleanup::Unconfirmed,
        crate::Cleanup::Joined { pid: 123 }] {
        let mut run = completed_run(failure, cleanup, false, false);
        assert_eq!(run.result().await.err(), Some(failure));
        assert_eq!(run.completion().wait().await, cleanup);
    }
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
