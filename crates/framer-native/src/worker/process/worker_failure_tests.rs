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

fn assert_stage(failure: Failure, label: &str) -> WorkerStage {
    let Failure::WorkerStage(stage) = failure else { panic!("missing safe stage") };
    assert_eq!(stage.as_str(), label);
    stage
}

fn completed_run(failure: Failure, cleanup: crate::worker::process::Cleanup, cancelled: bool,
    expired: bool) -> crate::worker::process::Run<()> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    assert!(sender.send(Err(failure)).is_ok());
    let (cancel, _receiver) = tokio::sync::watch::channel(cancelled);
    crate::worker::process::Run {
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(
            if expired { 0 } else { 3600 }),
        pid: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
        result: Some(receiver),
        completion: crate::worker::process::Completion(futures::future::ready(cleanup).boxed().shared()),
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
        assert!(assert_stage(actual, label).summary().is_none());
    }
}

#[test]
fn full_failure_retains_stage_for_stateless_and_device_modes() {
    let mut value = full();
    assert!(assert_stage(observed(&value, Some(2)), "mint").summary().is_some());
    value["version"] = json!(2);
    value["device_storage_seeded"] = json!(true);
    value["device_storage_unchanged"] = json!(false);
    value["failure_phase"] = json!("device-storage");
    assert!(assert_stage(observed(&value, Some(2)), "device-storage").summary().is_some());
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
    assert_stage(from_exit(Some(2), &bytes), "mint");
    assert!(crate::worker::process::metadata::accept(&bytes, 16000).is_none());
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
            assert_stage(from_exit(Some(2), &bytes), label);
            assert!(crate::worker::process::metadata::accept(&bytes, 16000).is_none());
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
        assert!(assert_stage(observed(&compact(label), Some(2)), label).summary().is_none());
        let mut value = full(); value["failure_phase"] = json!(label);
        assert!(assert_stage(observed(&value, Some(2)), label).summary().is_some());
    }
}

#[tokio::test]
async fn v3_phase_keeps_owner_cancellation_and_deadline_precedence() {
    let mut value = full(); value["failure_phase"] = json!("v3-sdk-error");
    let failure = observed(&value, Some(2));
    for (cancelled, expired, expected) in [(true, false, Failure::Cancelled),
        (true, true, Failure::Cancelled), (false, true, Failure::Deadline)] {
        let mut run = completed_run(failure, crate::worker::process::Cleanup::Unconfirmed, cancelled, expired);
        assert_eq!(run.result().await.err(), Some(expected));
        assert_eq!(run.completion().wait().await, crate::worker::process::Cleanup::Unconfirmed);
    }
}

#[tokio::test]
async fn v3_phase_retains_the_original_cleanup_verdict() {
    let mut value = full(); value["failure_phase"] = json!("v3-sdk-error");
    let failure = observed(&value, Some(2));
    for cleanup in [crate::worker::process::Cleanup::NotStarted, crate::worker::process::Cleanup::Unconfirmed,
        crate::worker::process::Cleanup::Joined { pid: 123 }] {
        let mut run = completed_run(failure, cleanup, false, false);
        assert_eq!(run.result().await.err(), Some(failure));
        assert_eq!(run.completion().wait().await, cleanup);
    }
}

fn handoff(version: u64, attempted: bool, completed: bool, age: Value) -> Value {
    let mut value = full();
    value["version"] = json!(version);
    value["failure_phase"] = json!("handoff");
    value["branch"] = json!({"v3":1,"v2":0,"none":0,"errors":0});
    value["token_pipe_write_attempted"] = json!(attempted);
    value["token_pipe_write_completed"] = json!(completed);
    value["mint_to_handoff_ms"] = age;
    if version == 2 {
        value["device_storage_seeded"] = json!(true);
        value["device_storage_unchanged"] = json!(true);
    }
    value
}

#[test]
fn handoff_retains_completed_facts_without_qualifying_token_or_parent_cleanup() {
    for version in [1, 2] {
        for (attempted, completed) in [(false, false), (true, false), (true, true)] {
            let value = handoff(version, attempted, completed, json!(2001.25));
            let stage = assert_stage(observed(&value, Some(2)), "handoff");
            let summary = stage.summary().unwrap();
            assert_eq!(summary.version(), version);
            assert_eq!(summary.flags().collect::<Vec<_>>(), FLAGS.into_iter()
                .map(|key| (key, value[key].as_bool().unwrap()))
                .chain((version == 2).then_some(DEVICE).into_iter().flatten().map(|key| (key, true)))
                .chain([(PIPE[0], attempted), (PIPE[1], completed)]).collect::<Vec<_>>());
            assert_eq!(summary.counts().collect::<Vec<_>>(), vec![("wrapper_calls", 1),
                ("dynamic_imports", 1), ("async_errors", 0), ("diagnostics", 1),
                ("late_settlements", 0), ("branch_v3", 1), ("branch_v2", 0),
                ("branch_none", 0), ("branch_errors", 0)]);
            assert_eq!(summary.timings().collect::<Vec<_>>(), vec![("method_ms", Some(3.0)),
                ("elapsed_ms", Some(5.0)), ("mint_to_handoff_ms", Some(2001.25))]);
        }
    }
}

#[test]
fn old_reports_do_not_invent_pipe_observations() {
    let stage = assert_stage(observed(&compact("handoff"), Some(2)), "handoff");
    assert!(stage.summary().is_none());
    let stage = assert_stage(observed(&full(), Some(2)), "mint");
    let summary = stage.summary().unwrap();
    assert_eq!(summary.flags().count(), FLAGS.len());
    assert!(!summary.flags().any(|(name, _)| PIPE.contains(&name)));
}

#[test]
fn handoff_unknown_age_is_explicit_and_requires_the_extended_schema() {
    for version in [1, 2] {
        let value = handoff(version, false, false, Value::Null);
        let stage = assert_stage(observed(&value, Some(2)), "handoff");
        assert_eq!(stage.summary().unwrap().timings().last(), Some(("mint_to_handoff_ms", None)));
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove("mint_to_handoff_ms");
        missing["private"] = json!(null);
        assert_eq!(observed(&missing, Some(2)), Failure::Worker);
    }
    let mut old = full(); old["mint_to_handoff_ms"] = Value::Null;
    assert_eq!(observed(&old, Some(2)), Failure::Worker);
}

#[test]
fn incomplete_handoff_summary_or_extra_private_field_stays_generic() {
    for version in [1, 2] {
        let value = handoff(version, true, true, json!(2001));
        for key in value.as_object().unwrap().keys() {
            let mut invalid = value.clone(); invalid.as_object_mut().unwrap().remove(key);
            assert_eq!(observed(&invalid, Some(2)), Failure::Worker, "{key}");
        }
        let mut invalid = value; invalid["token"] = json!("private-fixture-marker");
        let failure = observed(&invalid, Some(2));
        assert_eq!(failure, Failure::Worker);
        assert!(!format!("{failure:?}").contains("private-fixture-marker"));
    }
}

#[test]
fn malformed_or_inconsistent_handoff_summary_stays_generic() {
    for (key, invalid) in [("token_pipe_write_attempted", json!(1)),
        ("token_pipe_write_completed", json!(null)), ("wrapper_calls", json!(1_000_001)),
        ("mint_to_handoff_ms", json!(-1)), ("mint_to_handoff_ms", json!("private")),
        ("elapsed_ms", json!(null)), ("window_closed", json!("true")),
        ("failure_phase", json!("mint")), ("token_handoff", json!(true))] {
        let mut value = handoff(1, true, true, json!(2001)); value[key] = invalid;
        assert_eq!(observed(&value, Some(2)), Failure::Worker, "{key}");
    }
    assert_eq!(observed(&handoff(1, false, true, json!(1)), Some(2)), Failure::Worker);
}

#[test]
fn completed_pipe_write_never_passes_success_or_unexpected_exit_guards() {
    for version in [1, 2] {
        let value = handoff(version, true, true, json!(2001));
        let bytes = serde_json::to_vec(&value).unwrap();
        for code in [None, Some(0), Some(1), Some(3)] {
            assert_eq!(from_exit(code, &bytes), Failure::Worker);
        }
        assert!(crate::worker::process::metadata::accept_mode(&bytes, 16000, version == 2).is_none());
    }
}

#[tokio::test]
async fn retained_handoff_summary_preserves_owner_guards_and_cleanup_verdicts() {
    let failure = observed(&handoff(2, true, true, json!(2001)), Some(2));
    for cleanup in [crate::worker::process::Cleanup::NotStarted,
        crate::worker::process::Cleanup::Unconfirmed, crate::worker::process::Cleanup::Joined { pid: 123 }] {
        for (cancelled, expired, expected) in [(false, false, failure),
            (true, false, Failure::Cancelled), (true, true, Failure::Cancelled), (false, true, Failure::Deadline)] {
            let mut run = completed_run(failure, cleanup, cancelled, expired);
            assert_eq!(run.result().await.err(), Some(expected));
            assert_eq!(run.completion().wait().await, cleanup);
        }
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
