use super::*;
use serde_json::json;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::path::PathBuf;
use std::time::Duration;
fn valid() -> Value {
    json!({"version":1,"kind":"framer-v2-handoff","status":"passed",
      "failure_phase":null,"token_handoff":true,"mint_to_handoff_ms":1,
      "operation":{"version":1,"sdk":"framer-embedded-v2","selection":"configured-api",
        "status":"passed","failure_phase":null,"device_failure":null,
        "configure_calls":1,"token_method_calls":1,"device_storage_seeded":true,
        "device_storage_unchanged":true,"window_closed":true,"sources_unchanged":true,
        "async_errors":0,"diagnostics":0,"late_settlements":0,"method_ms":2,"elapsed_ms":5}})
}
fn bytes(value: &Value) -> Vec<u8> { serde_json::to_vec(value).unwrap() }
#[test]
fn requires_full_handoff_and_explicit_device() {
    assert!(accept(&bytes(&valid()), 16000, true).is_some());
    assert!(accept(&bytes(&valid()), 16000, false).is_none());
    assert!(accept(&bytes(&valid()["operation"]), 16000, true).is_none());
    assert!(super::super::metadata::accept_mode(&bytes(&valid()), 16000, true).is_none());
}
#[test]
fn changed_cleanup_and_stale_handoff_refuse() {
    for key in ["window_closed", "sources_unchanged", "device_storage_unchanged"] {
        let mut value = valid(); value["operation"][key] = json!(false);
        assert!(accept(&bytes(&value), 16000, true).is_none());
    }
    for age in [-1.0, 2000.1] {
        let mut value = valid(); value["mint_to_handoff_ms"] = json!(age);
        assert!(accept(&bytes(&value), 16000, true).is_none());
    }
}
#[test]
fn raw_duplicate_unknown_and_escaped_keys_refuse() {
    let raw = String::from_utf8(bytes(&valid())).unwrap();
    for value in [raw.replacen("{", "{\"kind\":\"foreign\",", 1),
        raw.replace("\"device_failure\":null", "\"device_failure\":\"local_value\",\"device_failure\":null"),
        raw.replace("\"device_failure\":null", "\"device_failure\":\"local_value\",\"device_\\u0066ailure\":null")] {
        assert!(accept(value.as_bytes(), 16000, true).is_none());
    }
    let mut value = valid(); value["raw"] = json!("private-fixture");
    assert!(accept(&bytes(&value), 16000, true).is_none());
}
fn target(mode: &str) -> Target {
    Target { project_id: mode.into(), path: "/ai/v3/responses/".into(),
        page_url: format!("https://framer.com/projects/{mode}") }
}
fn worker() -> V2Worker {
    let node = crate::test_support::node();
    let entry = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference/fixtures/offline_v2_worker_fake.mjs");
    let pins = [&node, &entry].into_iter().map(|path| super::super::FilePin {
        path: path.clone(), sha256: Sha256::digest(std::fs::read(path).unwrap()).into(),
    }).collect();
    V2Worker::new(Specification { node, entry, pins }).unwrap()
}
#[tokio::test(flavor = "current_thread")]
async fn private_v2_result_requires_child_join() {
    let device = SavedDevice::new("fixture-device".into()).unwrap();
    let mut run = worker().begin_with_device(target("success"), &device,
        Instant::now() + Duration::from_secs(5)).unwrap();
    let completion = run.completion(); let minted = run.result().await.unwrap();
    assert!(minted.freshness_origin <= minted.received_at);
    assert_eq!(minted.metadata.operation.method_ms, 2.0);
    assert_eq!(minted.into_token(), "fake-fixture-token");
    assert!(matches!(completion.wait().await, super::super::Cleanup::Joined { .. }));
}
#[tokio::test(flavor = "current_thread")]
async fn valid_v2_frame_cannot_override_failed_exit() {
    let device = SavedDevice::new("fixture-device".into()).unwrap();
    let mut run = worker().begin_with_device(target("failed"), &device,
        Instant::now() + Duration::from_secs(5)).unwrap();
    let completion = run.completion();
    assert!(matches!(run.result().await, Err(Failure::Worker)));
    assert!(matches!(completion.wait().await, super::super::Cleanup::Joined { .. }));
}
#[tokio::test(flavor = "current_thread")]
async fn cancelled_v2_keeps_cleanup_owner() {
    let device = SavedDevice::new("fixture-device".into()).unwrap();
    let run = worker().begin_with_device(target("hang"), &device,
        Instant::now() + Duration::from_secs(5)).unwrap();
    let pid = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(pid) = run.started_pid() { break pid; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap();
    let completion = run.completion(); drop(run);
    assert_eq!(completion.wait().await, super::super::Cleanup::Joined { pid });
}
#[tokio::test(flavor = "current_thread")]
async fn buffered_v2_frame_does_not_reset_freshness() {
    let mut worker = worker(); worker.0.first_read_delay = Some(Duration::from_millis(60));
    let device = SavedDevice::new("fixture-device".into()).unwrap();
    let mut run = worker.begin_with_device(target("success"), &device,
        Instant::now() + Duration::from_secs(5)).unwrap();
    let minted = run.result().await.unwrap();
    assert!(minted.received_at.duration_since(minted.freshness_origin)
        >= Duration::from_millis(60));
}
#[test]
fn failed_v2_envelope_retains_only_fixed_categories() {
    let mut value = valid(); value["status"] = json!("failed");
    value["failure_phase"] = json!("operation"); value["token_handoff"] = json!(false);
    value["operation"]["status"] = json!("failed");
    value["operation"]["failure_phase"] = json!("device-storage");
    value["operation"]["device_failure"] = json!("cookie_count");
    let Failure::V2WorkerStage(stage) = failure::from_exit(Some(2), &bytes(&value)) else { panic!() };
    assert_eq!(stage.phase(), "operation");
    assert_eq!(stage.operation(), Some("device-storage"));
    assert_eq!(stage.device(), Some("cookie_count"));
    assert_eq!(failure::from_exit(Some(3), &bytes(&value)), Failure::Worker);
    value["operation"]["device_failure"] = json!("private-value");
    assert_eq!(failure::from_exit(Some(2), &bytes(&value)), Failure::Worker);
}
#[tokio::test(flavor = "current_thread")]
async fn failed_v2_reply_keeps_cause_after_child_join() {
    let device = SavedDevice::new("fixture-device".into()).unwrap();
    let mut run = worker().begin_with_device(target("refused"), &device,
        Instant::now() + Duration::from_secs(5)).unwrap();
    let completion = run.completion();
    let Err(Failure::V2WorkerStage(stage)) = run.result().await else { panic!() };
    assert_eq!(stage.device(), Some("cookie_count"));
    assert!(matches!(completion.wait().await, super::super::Cleanup::Joined { .. }));
}
