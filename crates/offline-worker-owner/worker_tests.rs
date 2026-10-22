use super::*;
use serde_json::{json, Value};

fn valid_metadata() -> Value {
    json!({"version":1,"status":"passed","failure_phase":null,
        "target_validated":true,"account_state_loaded":false,"wrapper_calls":1,
        "branch":{"v3":1,"v2":0,"none":0,"errors":0},"dynamic_imports":1,
        "v3_evaluated":true,"window_closed":true,"inspector_closed":true,
        "sources_unchanged":true,"async_errors":0,"diagnostics":0,
        "late_settlements":0,"method_ms":2.0,"elapsed_ms":5.0,
        "mint_to_handoff_ms":1.0,"token_handoff":true})
}

#[test]
fn exact_success_metadata_is_accepted() {
    let bytes = serde_json::to_vec(&valid_metadata()).unwrap();
    assert!(metadata::accept(&bytes, 16000).is_some());
}
#[test]
fn missing_cleanup_is_rejected() {
    for field in ["window_closed", "inspector_closed", "sources_unchanged"] {
        let mut value = valid_metadata(); value[field] = json!(false);
        assert!(metadata::accept(&serde_json::to_vec(&value).unwrap(), 16000).is_none());
    }
}
#[test]
fn v2_fallback_is_rejected() {
    let mut value = valid_metadata(); value["branch"]["v2"] = json!(1);
    assert!(metadata::accept(&serde_json::to_vec(&value).unwrap(), 16000).is_none());
}
#[test]
fn invented_account_continuity_is_rejected() {
    let mut value = valid_metadata(); value["account_state_loaded"] = json!(true);
    assert!(metadata::accept(&serde_json::to_vec(&value).unwrap(), 16000).is_none());
}
#[test]
fn late_handoff_is_rejected() {
    let mut value = valid_metadata(); value["mint_to_handoff_ms"] = json!(2000.1);
    assert!(metadata::accept(&serde_json::to_vec(&value).unwrap(), 16000).is_none());
}
#[test]
fn unknown_metadata_cannot_carry_private_values() {
    let mut value = valid_metadata(); value["raw_token"] = json!("fake fixture");
    assert!(metadata::accept(&serde_json::to_vec(&value).unwrap(), 16000).is_none());
}

fn frame(value: &[u8]) -> Vec<u8> {
    let mut frame = b"FSDK\x01\0\0\0".to_vec();
    frame.extend_from_slice(&(value.len() as u32).to_be_bytes());
    frame.extend_from_slice(value); frame
}
#[test]
fn exact_single_frame_is_accepted() {
    assert_eq!(metadata::token(frame(b"fake")), Some("fake".to_owned()));
}
#[test]
fn truncated_extra_and_invalid_utf8_frames_are_rejected() {
    let mut truncated = frame(b"fake"); truncated.pop();
    let mut extra = frame(b"fake"); extra.push(0);
    for bytes in [truncated, extra, frame(&[0xff]), frame(&[])] {
        assert!(metadata::token(bytes).is_none());
    }
}
#[test]
fn oversized_frame_is_rejected() {
    assert!(metadata::token(frame(&vec![b'x'; 16385])).is_none());
}

fn inert_spec() -> Specification {
    let node = PathBuf::from("/does-not-exist/offline-node");
    let entry = PathBuf::from("/does-not-exist/offline-worker.mjs");
    Specification { node: node.clone(), entry: entry.clone(), pins: vec![
        FilePin { path: node, sha256: [0; 32] }, FilePin { path: entry, sha256: [0; 32] },
    ] }
}
#[test]
fn construction_performs_no_file_or_process_work() {
    assert!(Worker::new(inert_spec()).is_ok());
}
#[test]
fn duplicate_pins_are_rejected() {
    let mut spec = inert_spec();
    spec.pins.push(FilePin { path: spec.node.clone(), sha256: [0; 32] });
    assert!(matches!(Worker::new(spec), Err(Failure::Configuration)));
}
fn target(mode: &str) -> Target {
    Target { project_id: mode.into(), path: "/ai/v3/responses/".into(),
        page_url: format!("https://framer.com/projects/{mode}") }
}
#[test]
fn expired_deadline_never_starts_work() {
    let worker = Worker::new(inert_spec()).unwrap();
    assert!(matches!(worker.begin(target("fake"), Instant::now()), Err(Failure::Deadline)));
}
#[test]
fn absent_runtime_never_starts_work() {
    let worker = Worker::new(inert_spec()).unwrap();
    assert!(matches!(worker.begin(target("fake"), Instant::now() + Duration::from_secs(1)),
        Err(Failure::Runtime)));
}

fn pin(path: PathBuf, expected: &str) -> FilePin {
    assert_eq!(expected.len(), 64);
    let mut sha256 = [0u8; 32];
    for (index, byte) in sha256.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&expected[index*2..index*2+2], 16).unwrap();
    }
    FilePin { path, sha256 }
}
fn fake_worker() -> Worker {
    let node = PathBuf::from("/home/tjninja/.nvm/versions/node/v24.13.0/bin/node");
    let base = PathBuf::from(concat!("/home/tjninja/.t3/scratch/",
        "2026-10-20-can-you-look-at-the-430fdc9c/browserless-offline-mint-r2"));
    let entry = base.join("fake_worker.mjs");
    let pins = vec![
        pin(node.clone(), "53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc"),
        pin(entry.clone(), "3933fdca4bde11c27b6174d08a5eb790047ccefbe9e89573cc1dc948d269a1bb"),
        pin(base.join("protocol.mjs"), "99117048db3589a307efcd11ce07960f8e80afc0aece3d1a4d000f55ec1622df"),
    ];
    Worker::new(Specification { node, entry, pins }).unwrap()
}
async fn begin_fake(mode: &str, seconds: u64) -> Run {
    fake_worker().begin(target(mode), Instant::now() + Duration::from_secs(seconds)).unwrap()
}
async fn started(run: &Run) -> u32 {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(pid) = run.started_pid() { return pid; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("fixture child must start")
}
#[tokio::test(flavor = "current_thread")]
async fn private_handoff_requires_child_and_task_join() {
    let mut run = begin_fake("success", 5).await;
    let completion = run.completion();
    let value = run.result().await.expect("fake worker must pass");
    assert!(value.received_at <= Instant::now());
    assert_eq!(value.into_token(), "fake-fixture-token");
    assert!(matches!(completion.wait().await, Cleanup::Joined { .. }));
}
#[tokio::test(flavor = "current_thread")]
async fn dropped_run_keeps_independent_cleanup() {
    let run = begin_fake("hang", 5).await;
    let pid = started(&run).await; let completion = run.completion();
    drop(run);
    assert_eq!(completion.wait().await, Cleanup::Joined { pid });
}
#[tokio::test(flavor = "current_thread")]
async fn dropped_result_wait_requests_cancellation() {
    let mut run = begin_fake("hang", 5).await;
    let pid = started(&run).await; let completion = run.completion();
    assert!(tokio::time::timeout(Duration::from_millis(20), run.result()).await.is_err());
    assert_eq!(completion.wait().await, Cleanup::Joined { pid });
}
#[tokio::test(flavor = "current_thread")]
async fn explicit_cancel_never_returns_a_token() {
    let mut run = begin_fake("hang", 5).await;
    let pid = started(&run).await; run.cancel();
    assert!(matches!(run.result().await, Err(Failure::Cancelled)));
    assert_eq!(run.completion().wait().await, Cleanup::Joined { pid });
}
#[tokio::test(flavor = "current_thread")]
async fn token_before_exit_does_not_complete_the_operation() {
    let mut run = begin_fake("linger", 2).await;
    let pid = started(&run).await;
    assert!(matches!(run.result().await, Err(Failure::Deadline)));
    assert_eq!(run.completion().wait().await, Cleanup::Joined { pid });
}
#[tokio::test(flavor = "current_thread")]
async fn malformed_frames_and_metadata_are_rejected_after_join() {
    for mode in ["truncated", "extra", "badmeta"] {
        let mut run = begin_fake(mode, 5).await;
        assert!(matches!(run.result().await, Err(Failure::Protocol)));
        assert!(matches!(run.completion().wait().await, Cleanup::Joined { .. }));
    }
}
#[tokio::test(flavor = "current_thread")]
async fn failed_exit_cannot_publish_a_valid_frame() {
    let mut run = begin_fake("fail", 5).await;
    assert!(matches!(run.result().await, Err(Failure::Worker)));
    assert!(matches!(run.completion().wait().await, Cleanup::Joined { .. }));
}
#[tokio::test(flavor = "current_thread")]
async fn oversized_pipe_output_is_killed_and_joined() {
    let mut run = begin_fake("oversized", 5).await;
    assert!(matches!(run.result().await, Err(Failure::Io)));
    assert!(matches!(run.completion().wait().await, Cleanup::Joined { .. }));
}
#[tokio::test(flavor = "current_thread")]
async fn kernel_denies_sockets_and_new_processes_in_child_only() {
    let before = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    assert!(before >= 0); unsafe { libc::close(before); }
    let mut run = begin_fake("sandbox", 5).await;
    assert_eq!(run.result().await.unwrap().into_token(), "fake-fixture-token");
    assert!(matches!(run.completion().wait().await, Cleanup::Joined { .. }));
    let after = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    assert!(after >= 0); unsafe { libc::close(after); }
}
#[tokio::test(flavor = "current_thread")]
async fn cancellation_before_admission_starts_no_child() {
    let mut run = begin_fake("hang", 5).await;
    run.cancel();
    assert!(matches!(run.result().await, Err(Failure::Cancelled)));
    assert_eq!(run.completion().wait().await, Cleanup::NotStarted);
    assert!(run.started_pid().is_none());
}
#[tokio::test(flavor = "current_thread")]
async fn replaced_input_is_rejected_before_launch() {
    let mut worker = fake_worker();
    Arc::get_mut(&mut worker.specification).unwrap().pins[0].sha256[0] ^= 1;
    let mut run = worker.begin(target("success"), Instant::now() + Duration::from_secs(5)).unwrap();
    assert!(matches!(run.result().await, Err(Failure::Identity)));
    assert_eq!(run.completion().wait().await, Cleanup::NotStarted);
}

#[tokio::test(flavor = "current_thread")]
async fn eof_and_join_do_not_renew_token_receipt_time() {
    for mode in ["delayedclose", "delayedexit"] {
        let mut run = begin_fake(mode, 5).await;
        let value = run.result().await.expect("delayed fake worker must pass");
        assert!(value.received_at.elapsed() >= Duration::from_millis(100));
        assert_eq!(value.into_token(), "fake-fixture-token");
        assert!(matches!(run.completion().wait().await, Cleanup::Joined { .. }));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_during_pin_verification_starts_no_child() {
    let mut run = begin_fake("success", 5).await;
    tokio::task::yield_now().await;
    run.cancel();
    assert!(matches!(run.result().await, Err(Failure::Cancelled)));
    assert_eq!(run.completion().wait().await, Cleanup::NotStarted);
    assert!(run.started_pid().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn non_regular_input_is_rejected_without_launch() {
    let mut spec = inert_spec();
    spec.node = PathBuf::from("/dev/null");
    spec.pins[0].path = spec.node.clone();
    let worker = Worker::new(spec).unwrap();
    let mut run = worker.begin(target("fake"), Instant::now() + Duration::from_secs(1)).unwrap();
    assert!(matches!(run.result().await, Err(Failure::Identity)));
    assert_eq!(run.completion().wait().await, Cleanup::NotStarted);
}

#[tokio::test(flavor = "current_thread")]
async fn delayed_first_read_keeps_pre_spawn_freshness_origin() {
    let mut worker = fake_worker();
    worker.first_read_delay = Some(Duration::from_millis(200));
    let mut run = worker.begin(target("success"), Instant::now() + Duration::from_secs(5)).unwrap();
    let completion = run.completion();
    let minted = run.result().await.expect("buffered fake worker must pass");
    let age_at_receipt = minted.received_at.duration_since(minted.freshness_origin);
    assert!(age_at_receipt >= Duration::from_millis(200));
    assert!(age_at_receipt > Duration::from_millis(100), "small test age budget expired");
    assert_eq!(minted.into_token(), "fake-fixture-token");
    assert!(matches!(completion.wait().await, Cleanup::Joined { .. }));
}
