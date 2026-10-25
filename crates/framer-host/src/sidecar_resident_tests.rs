use super::*;
use super::super::{PendingReplies, Sidecar};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{atomic::{AtomicBool, AtomicU64}, Mutex as StdMutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};

const ACCOUNT: &str = "00000000-0000-4000-8000-000000000001";
const LEASE: &str = "00000000-0000-4000-8000-000000000002";
const PROVENANCE: &str = "00000000-0000-4000-8000-000000000003";

#[tokio::test]
async fn sdk_phases_consume_native_branch_and_keep_original_deadline() {
    let mut f = fixture(Some(Ok(json!(true)))).await;
    let mut rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap().into_v2();
    assert!(matches!(rpc.begin_native_sdk().await, Err(ResidentError::OutOfOrder)));
    rpc.acquire(acquire(45_000).unwrap(), PROVENANCE).await.unwrap();
    rpc.baseline("fixture-bearer", &json!({})).await.unwrap();
    let deadline = rpc.deadline();
    assert!(matches!(rpc.complete_native_sdk().await, Err(ResidentError::OutOfOrder)));
    rpc.begin_native_sdk().await.unwrap();
    assert!(matches!(rpc.mint_native().await, Err(ResidentError::OutOfOrder)));
    assert!(matches!(rpc.begin_native_sdk().await, Err(ResidentError::OutOfOrder)));
    rpc.complete_native_sdk().await.unwrap();
    assert_eq!(rpc.deadline(), deadline);
    assert!(matches!(rpc.complete_native_sdk().await, Err(ResidentError::OutOfOrder)));
    rpc.finish(ResidentAdmission::Fenced).await.unwrap();
    let actions = ["acquire", "baseline", "begin_native_sdk", "complete_native_sdk", "finish"];
    for action in actions { assert_eq!(f.frames.try_recv().unwrap()["action"], action); }
    assert!(f.frames.try_recv().is_err());
    close(&f.owner, f.reader).await;
}

fn expected_v2_frame(id: u64, extra: Value) -> Value {
    let mut frame = expected_frame(id, extra);
    frame["version"] = json!(2);
    frame
}

fn lossy_baseline_numbers() -> Vec<Value> {
    let mut numbers = vec![json!(9_007_199_254_740_992_u64),
        json!(-9_007_199_254_740_992_i64), json!(9_007_199_254_740_993_u64),
        json!(-9_007_199_254_740_993_i64), json!(u64::MAX), json!(i64::MIN)];
    for spelling in ["9007199254740992.0", "-9007199254740992.0",
        "9007199254740993.0", "-9007199254740993.0",
        "9.007199254740993e15", "-9.007199254740993e15",
        "9007199254740992.5", "-9007199254740992.5", "1e100", "-1e100"] {
        numbers.push(serde_json::from_str(spelling).unwrap());
    }
    numbers
}

#[test]
fn v2_baseline_body_checks_serialized_bytes_and_nested_tokens() {
    let at_limit = json!({"x": "a".repeat(64 * 1024 - 8)});
    assert_eq!(serde_json::to_vec(&at_limit).unwrap().len(), 64 * 1024);
    assert!(baseline_body(&at_limit));
    assert!(!baseline_body(&json!({"x": "a".repeat(64 * 1024 - 7)})));
    assert!(baseline_body(&json!({"x": "é".repeat((64 * 1024 - 8) / 2)})));
    assert!(!baseline_body(&json!({"x": "é".repeat((64 * 1024 - 8) / 2 + 1)})));
    assert!(!baseline_body(&json!({"x": "\n".repeat(64 * 1024 / 2)})));
    for invalid in [Value::Null, json!([]), json!("object required"),
        json!({"castleToken": null}), json!({"nested": [{"castleToken": "fixture"}]})] {
        assert!(!baseline_body(&invalid));
    }
    for number in lossy_baseline_numbers() {
        let body = json!({"request": {"tools": [{"parameters": {"minimum": number}}]}});
        assert_eq!(ResidentRpcV2::validate_baseline_body(&body),
            Err(ResidentError::InvalidBaseline));
    }
    for spelling in ["-9007199254740991", "0", "9007199254740991",
        "-9007199254740991.0", "9007199254740991.0",
        "-9.007199254740991e15", "9.007199254740991e15",
        "0.25", "-0.25", "0.1", "-0.0", "true", "false", "null"] {
        let value: Value = serde_json::from_str(spelling).unwrap();
        let body = json!({"request": {"tools": [{"parameters": {"minimum": value}}]}});
        let before = body.clone();
        assert_eq!(ResidentRpcV2::validate_baseline_body(&body), Ok(()));
        assert_eq!(body, before);
    }
    let mut nested = json!({});
    for _ in 0..65 { nested = json!({"x": nested}); }
    assert!(!baseline_body(&nested));
    let mut final_body = json!({"turnId": "n", "padding": ""});
    let overhead = serde_json::to_vec(&final_body).unwrap().len();
    final_body["padding"] = json!("a".repeat(64 * 1024 - overhead));
    assert_eq!(ResidentRpcV2::validate_baseline_body(&final_body), Ok(()));
    final_body["turnId"] = json!("longer-browser-baseline-turn");
    assert_eq!(ResidentRpcV2::validate_baseline_body(&final_body),
        Err(ResidentError::InvalidBaseline));
}

#[tokio::test]
async fn v2_emits_exact_ordered_frames_without_resetting_deadline() {
    let mut f = fixture(Some(Ok(json!(true)))).await;
    let mut rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap().into_v2();
    let _ = rpc.acquire(acquire(45_000).unwrap(), PROVENANCE).await.unwrap();
    let deadline = rpc.deadline().unwrap();
    let body = json!({"turnId": "baseline-turn", "stream": false,
        "messages": [{"role": "user", "content": "synthetic π"}]});
    let _ = rpc.baseline("synthetic-bearer", &body).await.unwrap();
    assert_eq!(rpc.deadline(), Some(deadline));
    let _ = rpc.mint_native().await.unwrap();
    assert_eq!(rpc.deadline(), Some(deadline));
    let _ = rpc.finish(ResidentAdmission::Fenced).await.unwrap();
    assert_eq!(rpc.deadline(), Some(deadline));
    let expected = [
        json!({"action": "acquire", "project": "project", "path": "/fixture",
            "wrapper_url": "https://fixture.invalid/wrapper.mjs", "wrapper_export": "b",
            "cookie_header": "session=synthetic-only", "budget_ms": 45_000,
            "provenance": PROVENANCE}),
        json!({"action": "baseline", "bearer": "synthetic-bearer", "body": body}),
        json!({"action": "mint_native"}),
        json!({"action": "finish", "native_state": "settled", "admission": "fenced"}),
    ];
    for (index, extra) in expected.into_iter().enumerate() {
        assert_eq!(f.frames.try_recv().unwrap(), expected_v2_frame(index as u64 + 1, extra));
    }
    assert!(f.frames.try_recv().is_err());
    close(&f.owner, f.reader).await;
}

#[tokio::test]
async fn v2_refuses_skipped_and_repeated_phases_without_writing() {
    let mut f = fixture(Some(Ok(json!(true)))).await;
    let mut rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap().into_v2();
    assert_eq!(rpc.baseline("fixture", &json!({})).await.err(), Some(ResidentError::OutOfOrder));
    assert_eq!(rpc.mint_native().await.err(), Some(ResidentError::OutOfOrder));
    assert_eq!(rpc.finish(ResidentAdmission::Fenced).await.err(), Some(ResidentError::OutOfOrder));
    assert_no_command(&mut f).await;
    let _ = rpc.acquire(acquire(45_000).unwrap(), PROVENANCE).await.unwrap();
    assert_eq!(rpc.acquire(acquire(45_000).unwrap(), PROVENANCE).await.err(),
        Some(ResidentError::OutOfOrder));
    assert_eq!(rpc.mint_native().await.err(), Some(ResidentError::OutOfOrder));
    let _ = rpc.baseline("fixture", &json!({})).await.unwrap();
    assert_eq!(rpc.baseline("fixture", &json!({})).await.err(), Some(ResidentError::OutOfOrder));
    let _ = rpc.mint_native().await.unwrap();
    assert_eq!(rpc.mint_native().await.err(), Some(ResidentError::OutOfOrder));
    let _ = rpc.finish(ResidentAdmission::Fenced).await.unwrap();
    assert_eq!(rpc.discard(ResidentDiscard::Unknown).await.err(), Some(ResidentError::OutOfOrder));
    assert_eq!(rpc.acquire(acquire(45_000).unwrap(), PROVENANCE).await.err(),
        Some(ResidentError::OutOfOrder));
    for action in ["acquire", "baseline", "mint_native", "finish"] {
        assert_eq!(f.frames.try_recv().unwrap()["action"], action);
    }
    assert!(f.frames.try_recv().is_err());
    close(&f.owner, f.reader).await;
}

#[tokio::test]
async fn v2_validates_private_inputs_before_any_command() {
    let mut f = fixture(Some(Ok(json!(true)))).await;
    let mut rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap().into_v2();
    for provenance in ["", "synthetic", "00000000-0000-0000-0000-000000000000",
        "00000000000040008000000000000003"] {
        assert_eq!(rpc.acquire(acquire(45_000).unwrap(), provenance).await.err(),
            Some(ResidentError::InvalidAcquire));
        assert!(rpc.deadline().is_none());
    }
    assert_no_command(&mut f).await;
    let _ = rpc.acquire(acquire(45_000).unwrap(), PROVENANCE).await.unwrap();
    f.frames.try_recv().unwrap();
    for bearer in [String::new(), "x".repeat(16 * 1024 + 1), "bad\nvalue".into()] {
        assert_eq!(rpc.baseline(&bearer, &json!({})).await.err(),
            Some(ResidentError::InvalidBaseline));
    }
    for body in [json!([]), json!({"castleToken": "fixture"}),
        json!({"x": "a".repeat(64 * 1024)})] {
        assert_eq!(rpc.baseline("fixture", &body).await.err(),
            Some(ResidentError::InvalidBaseline));
    }
    for number in lossy_baseline_numbers() {
        let body = json!({"request": {"tools": [{"parameters": {"minimum": number}}]}});
        assert_eq!(rpc.baseline("fixture", &body).await.err(),
            Some(ResidentError::InvalidBaseline));
    }
    assert!(f.frames.try_recv().is_err());
    assert!(rpc.phase == V2Phase::Acquired);
    let _ = rpc.baseline(&"x".repeat(16 * 1024), &json!({})).await.unwrap();
    assert_eq!(f.frames.try_recv().unwrap()["action"], "baseline");
    close(&f.owner, f.reader).await;
}

#[tokio::test]
async fn v2_expiry_refuses_new_work_but_still_allows_fenced_settlement() {
    let mut f = fixture(Some(Ok(json!(true)))).await;
    let mut rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap().into_v2();
    let _ = rpc.acquire(acquire(45_000).unwrap(), PROVENANCE).await.unwrap();
    f.frames.try_recv().unwrap();
    let expired = tokio::time::Instant::now() - Duration::from_secs(1);
    rpc.deadline = Some(expired);
    assert_eq!(rpc.baseline("fixture", &json!({})).await.err(), Some(ResidentError::Deadline));
    assert_eq!(rpc.deadline(), Some(expired));
    assert_eq!(rpc.mint_native().await.err(), Some(ResidentError::OutOfOrder));
    assert!(f.frames.try_recv().is_err());
    let _ = rpc.discard(ResidentDiscard::Unknown).await.unwrap();
    assert_eq!(f.frames.try_recv().unwrap(), expected_v2_frame(2,
        json!({"action": "discard", "native_state": "unknown"})));
    assert_eq!(rpc.deadline(), Some(expired));
    close(&f.owner, f.reader).await;
}

#[tokio::test]
async fn v2_failed_acquire_is_not_retryable_and_never_unlocks_baseline() {
    let mut f = fixture(Some(Err("synthetic private error".into()))).await;
    let mut rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap().into_v2();
    assert_eq!(rpc.acquire(acquire(45_000).unwrap(), PROVENANCE).await.err(),
        Some(ResidentError::Unconfirmed));
    let deadline = rpc.deadline();
    assert_eq!(rpc.acquire(acquire(45_000).unwrap(), PROVENANCE).await.err(),
        Some(ResidentError::OutOfOrder));
    assert_eq!(rpc.baseline("fixture", &json!({})).await.err(), Some(ResidentError::OutOfOrder));
    assert_eq!(rpc.mint_native().await.err(), Some(ResidentError::OutOfOrder));
    assert_eq!(rpc.deadline(), deadline);
    assert_eq!(f.frames.try_recv().unwrap()["action"], "acquire");
    assert!(f.frames.try_recv().is_err());
    assert_eq!(rpc.discard(ResidentDiscard::Unknown).await.err(), Some(ResidentError::Unconfirmed));
    assert_eq!(f.frames.try_recv().unwrap()["action"], "discard");
    assert_eq!(rpc.finish(ResidentAdmission::Fenced).await.err(), Some(ResidentError::OutOfOrder));
    assert!(f.frames.try_recv().is_err());
    close(&f.owner, f.reader).await;
}

async fn answer_next(f: &mut Fixture,
    work: impl std::future::Future<Output = Result<ResidentReply, ResidentError>>,
    reply: Result<Value, String>) -> Result<ResidentReply, ResidentError> {
    tokio::pin!(work);
    let frame = tokio::select! {
        _ = &mut work => panic!("silent fixture answered without injection"),
        frame = f.frames.recv() => frame.unwrap(),
        _ = tokio::time::sleep(Duration::from_secs(2)) => panic!("no fixture frame"),
    };
    let pending = f.owner.inner.lock().await.as_ref().unwrap().pending.clone();
    let sender = pending.lock().unwrap().remove(&frame["id"].as_u64().unwrap()).unwrap();
    sender.send(reply).unwrap();
    tokio::time::timeout(Duration::from_secs(2), work).await.unwrap()
}

#[tokio::test]
async fn v2_failed_baseline_and_mint_never_replay_the_phase() {
    for failed_mint in [false, true] {
        let mut f = fixture(None).await;
        let mut rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap().into_v2();
        let _ = answer_next(&mut f, rpc.acquire(acquire(45_000).unwrap(), PROVENANCE),
            Ok(json!(true))).await.unwrap();
        if failed_mint {
            let _ = answer_next(&mut f, rpc.baseline("fixture", &json!({})),
                Ok(json!(true))).await.unwrap();
            assert_eq!(answer_next(&mut f, rpc.mint_native(), Err("private".into())).await.err(),
                Some(ResidentError::Unconfirmed));
        } else {
            assert_eq!(answer_next(&mut f, rpc.baseline("fixture", &json!({})),
                Err("private".into())).await.err(), Some(ResidentError::Unconfirmed));
        }
        assert_eq!(rpc.baseline("fixture", &json!({})).await.err(), Some(ResidentError::OutOfOrder));
        assert_eq!(rpc.mint_native().await.err(), Some(ResidentError::OutOfOrder));
        assert!(f.frames.try_recv().is_err());
        let _ = answer_next(&mut f, rpc.discard(ResidentDiscard::Unknown),
            Ok(json!(true))).await.unwrap();
        close(&f.owner, f.reader).await;
    }
}

#[tokio::test]
async fn v2_dropped_baseline_waiter_stays_consumed_and_requires_settlement() {
    let mut f = fixture(None).await;
    let mut rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap().into_v2();
    let _ = answer_next(&mut f, rpc.acquire(acquire(45_000).unwrap(), PROVENANCE),
        Ok(json!(true))).await.unwrap();
    let body = json!({"stream": false});
    let mut waiter = Box::pin(rpc.baseline("fixture", &body));
    let frame = tokio::select! {
        _ = &mut waiter => panic!("silent peer answered"),
        frame = f.frames.recv() => frame.unwrap(),
        _ = tokio::time::sleep(Duration::from_secs(2)) => panic!("no baseline frame"),
    };
    let id = frame["id"].as_u64().unwrap();
    let pending = f.owner.inner.lock().await.as_ref().unwrap().pending.clone();
    assert!(pending.lock().unwrap().contains_key(&id));
    drop(waiter);
    assert!(!pending.lock().unwrap().contains_key(&id));
    assert_eq!(f.cancellations.try_recv().unwrap(), id);
    assert_eq!(rpc.baseline("fixture", &body).await.err(), Some(ResidentError::OutOfOrder));
    assert_eq!(rpc.mint_native().await.err(), Some(ResidentError::OutOfOrder));
    assert!(f.frames.try_recv().is_err());
    let _ = answer_next(&mut f, rpc.discard(ResidentDiscard::Unknown),
        Ok(json!(true))).await.unwrap();
    close(&f.owner, f.reader).await;
}

#[tokio::test]
async fn bootstrap_binding_rejects_replacement_without_command_or_rebinding() {
    let mut f = fixture(Some(Ok(json!(true)))).await;
    let captured = f.owner.inner.lock().await.as_ref().unwrap().incarnation;
    let rpc = ResidentRpc::bind_existing(&f.owner, identity(), captured).await.unwrap();
    assert_eq!(rpc.sidecar_incarnation, captured);
    assert_eq!(ResidentRpc::bind_existing(&f.owner, identity(), Uuid::nil()).await.err(),
        Some(ResidentError::InvalidIdentity));
    let replacement = Uuid::new_v4();
    f.owner.inner.lock().await.as_mut().unwrap().incarnation = replacement;
    assert_eq!(ResidentRpc::bind_existing(&f.owner, identity(), captured).await.err(),
        Some(ResidentError::Unconfirmed));
    assert_eq!(rpc.acquire(acquire(45_000).unwrap()).await.err(),
        Some(ResidentError::Unconfirmed));
    let compatible = ResidentRpc::bind(&f.owner, identity()).await.unwrap();
    assert_eq!(compatible.sidecar_incarnation, replacement);
    assert_no_command(&mut f).await;
    close(&f.owner, f.reader).await;
}

fn identity() -> ResidentIdentity {
    ResidentIdentity::new(AccountId(Uuid::parse_str(ACCOUNT).unwrap()),
        ApplicationGeneration(37), "python-generation".into(), 9,
        Uuid::parse_str(LEASE).unwrap()).unwrap()
}

fn acquire(budget_ms: u64) -> Result<ResidentAcquire, ResidentError> {
    ResidentAcquire::new("project".into(), "/fixture".into(),
        "https://fixture.invalid/wrapper.mjs".into(), "b".into(),
        "session=synthetic-only".into(), budget_ms)
}

#[test]
fn acquire_budget_and_private_fields_are_bounded_before_serialization() {
    for budget in [0, 45_001, u64::MAX] {
        assert!(matches!(acquire(budget), Err(ResidentError::InvalidAcquire)));
    }
    for budget in [1, 45_000] { assert!(acquire(budget).is_ok()); }
    let caps = [128, 8192, 8192, 128, 16_384];
    let request = |values: &[String; 5]| ResidentAcquire::new(values[0].clone(),
        values[1].clone(), values[2].clone(), values[3].clone(), values[4].clone(), 1);
    let values = caps.map(|cap| "x".repeat(cap));
    assert!(request(&values).is_ok());
    for (field, cap) in caps.into_iter().enumerate() {
        for invalid in [String::new(), "x".repeat(cap + 1), "a\r\nb".into()] {
            let mut invalid_values = values.clone();
            invalid_values[field] = invalid;
            assert!(matches!(request(&invalid_values), Err(ResidentError::InvalidAcquire)));
        }
    }
}

#[test]
fn common_identity_rejects_missing_or_unbounded_identity() {
    let account = AccountId(Uuid::parse_str(ACCOUNT).unwrap());
    let lease = Uuid::parse_str(LEASE).unwrap();
    assert!(matches!(ResidentIdentity::new(account, ApplicationGeneration(0),
        "python-generation".into(), 9, lease), Err(ResidentError::InvalidIdentity)));
    assert!(ResidentIdentity::new(account, ApplicationGeneration(u64::MAX),
        "python-generation".into(), 9, lease).is_ok());
    for host in [String::new(), "x".repeat(129), "host\n".into()] {
        assert!(matches!(ResidentIdentity::new(account, ApplicationGeneration(37),
            host, 9, lease), Err(ResidentError::InvalidIdentity)));
    }

    for (account, incarnation, lease) in [
        (AccountId(Uuid::nil()), 9, lease), (account, 0, lease), (account, 9, Uuid::nil()),
    ] {
        assert!(matches!(ResidentIdentity::new(account, ApplicationGeneration(37),
            "python-generation".into(), incarnation, lease), Err(ResidentError::InvalidIdentity)));
    }
    assert!(ResidentIdentity::new(account, ApplicationGeneration(37),
        "x".repeat(128), 9, lease).is_ok());
}

// Inspect command construction only, without changing the parent environment.
fn assert_child_generation(bridge: &SidecarBridge, expected: Option<&str>) {
    use std::ffi::OsStr;
    let key = OsStr::new(super::super::RESIDENT_GENERATION_ENV);
    let mut command = Command::new("never-launched-resident-fixture");
    command.env(key, "999");
    bridge.configure_resident_environment(&mut command);
    assert_eq!(command.as_std().get_envs().collect::<Vec<_>>(),
        vec![(key, expected.map(OsStr::new))]);
}

#[test]
fn default_bridge_disables_resident_controls_and_removes_ambient_generation() {
    let root = std::env::temp_dir().join(format!("uncreated-resident-{}", Uuid::new_v4()));
    let bridge = SidecarBridge::new(&root);
    assert_eq!(bridge.resident_application_generation, None);
    assert_child_generation(&bridge, None);
    assert!(bridge.inner.try_lock().unwrap().is_none());
    assert!(bridge.startup_attempted.get().is_none());
    assert!(!root.exists());
}

#[test]
fn explicit_positive_generation_replaces_ambient_value_without_starting() {
    for generation in [1, u64::MAX] {
        let mut bridge = SidecarBridge::new("unused-resident-fixture");
        bridge.configure_resident_controls(ApplicationGeneration(generation)).unwrap();
        assert_eq!(bridge.resident_application_generation,
            Some(ApplicationGeneration(generation)));
        assert_child_generation(&bridge, Some(&generation.to_string()));
        assert!(bridge.inner.try_lock().unwrap().is_none());
        assert!(bridge.startup_attempted.get().is_none());
    }
}

#[test]
fn zero_generation_never_enables_or_replaces_resident_configuration() {
    let mut bridge = SidecarBridge::new("unused-resident-fixture");
    assert!(bridge.configure_resident_controls(ApplicationGeneration(0)).is_err());
    assert_eq!(bridge.resident_application_generation, None);
    bridge.configure_resident_controls(ApplicationGeneration(37)).unwrap();
    assert!(bridge.configure_resident_controls(ApplicationGeneration(0)).is_err());
    assert_eq!(bridge.resident_application_generation, Some(ApplicationGeneration(37)));
}

#[test]
fn attempted_startup_freezes_resident_configuration_even_without_a_child() {
    for generation in [None, Some(ApplicationGeneration(37))] {
        let mut bridge = SidecarBridge::new("unused-resident-fixture");
        if let Some(generation) = generation {
            bridge.configure_resident_controls(generation).unwrap();
        }
        bridge.startup_attempted.set(()).unwrap();
        assert!(bridge.configure_resident_controls(ApplicationGeneration(38)).is_err());
        assert_eq!(bridge.resident_application_generation, generation);
        assert!(bridge.inner.try_lock().unwrap().is_none());
    }
}

#[test]
fn observed_mode_cannot_opt_into_resident_controls() {
    let mut bridge = SidecarBridge::new("unused-resident-fixture");
    bridge.mode = super::super::observer::ProcessMode::Observed {
        script: "unused-observed-fixture.py".into(), profile_nonce: Uuid::new_v4(),
    };
    assert!(bridge.configure_resident_controls(ApplicationGeneration(37)).is_err());
    assert_eq!(bridge.resident_application_generation, None);
    assert_child_generation(&bridge, None);
}


struct Fixture {
    owner: Arc<SidecarBridge>,
    frames: mpsc::UnboundedReceiver<Value>,
    cancellations: mpsc::Receiver<u64>,
    reader: tokio::task::JoinHandle<()>,
}

// Synthetic pipe peer only: no host script, browser, SDK, account or network.
async fn fixture(reply: Option<Result<Value, String>>) -> Fixture {
    fixture_with_generation(reply, Some(ApplicationGeneration(37))).await
}

async fn fixture_with_generation(reply: Option<Result<Value, String>>,
    generation: Option<ApplicationGeneration>) -> Fixture {
    let mut bridge = SidecarBridge::new("unused-resident-fixture");
    if let Some(generation) = generation {
        bridge.configure_resident_controls(generation).unwrap();
    }
    let mut child = Command::new("python3").args(["-u", "-c",
        "import sys\nfor line in sys.stdin:\n sys.stdout.write(line)\n sys.stdout.flush()"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
    let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    let stdout = child.stdout.take().unwrap();
    let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
    let observed = pending.clone();
    let (capture, frames) = mpsc::unbounded_channel();
    let reader = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let value: Value = serde_json::from_str(&line).unwrap();
            let id = value["id"].as_u64().unwrap();
            capture.send(value).unwrap();
            if let Some(reply) = &reply {
                if let Some(tx) = observed.lock().unwrap().remove(&id) {
                    let _ = tx.send(reply.clone());
                }
            }
        }
    });
    let (cancel, cancellations) = mpsc::channel(8);
    let owner = Arc::new(bridge);
    *owner.inner.lock().await = Some(Sidecar {
        child, stdin, pending, cancel, next_id: Arc::new(AtomicU64::new(0)),
        capability: Arc::new(Mutex::new(None)),
        healthy: Arc::new(AtomicBool::new(true)), incarnation: Uuid::new_v4(),
        events: tokio::sync::broadcast::channel(8).0,
    });
    Fixture { owner, frames, cancellations, reader }
}

async fn close(owner: &SidecarBridge, reader: tokio::task::JoinHandle<()>) {
    let mut inner = owner.inner.lock().await;
    let child = &mut inner.as_mut().unwrap().child;
    child.kill().await.unwrap();
    child.wait().await.unwrap();
    drop(inner);
    tokio::time::timeout(Duration::from_secs(2), reader).await.unwrap().unwrap();
}

async fn assert_no_command(f: &mut Fixture) {
    let inner = f.owner.inner.lock().await;
    let host = inner.as_ref().unwrap();
    assert_eq!(host.next_id.load(Ordering::Relaxed), 0);
    assert!(host.pending.lock().unwrap().is_empty());
    assert!(f.frames.try_recv().is_err());
    assert!(f.cancellations.try_recv().is_err());
}

fn expected_frame(id: u64, extra: Value) -> Value {
    let mut expected = json!({"version": 1, "id": id, "cmd": "resident_authorization_once",
        "account": ACCOUNT, "application_generation": 37,
        "host_generation": "python-generation", "account_incarnation": 9,
        "lease": LEASE});
    expected.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    expected
}

#[tokio::test]
async fn disabled_and_mismatched_generations_cannot_bind_a_running_sidecar() {
    for generation in [None, Some(ApplicationGeneration(38))] {
        let mut f = fixture_with_generation(Some(Ok(json!(true))), generation).await;
        let expected = if generation.is_some() { ResidentError::InvalidIdentity }
            else { ResidentError::Unconfirmed };
        assert_eq!(ResidentRpc::bind(&f.owner, identity()).await.err(), Some(expected));
        assert_no_command(&mut f).await;
        close(&f.owner, f.reader).await;
    }
}

#[tokio::test]
async fn late_opt_in_retains_the_existing_bridge_and_child() {
    let mut f = fixture_with_generation(Some(Ok(json!(true))), None).await;
    let bridge = Arc::get_mut(&mut f.owner).unwrap();
    let pid = bridge.inner.get_mut().as_ref().unwrap().child.id();
    assert!(bridge.configure_resident_controls(ApplicationGeneration(37)).is_err());
    assert_eq!(bridge.resident_application_generation, None);
    assert_eq!(bridge.inner.get_mut().as_ref().unwrap().child.id(), pid);
    assert_no_command(&mut f).await;
    close(&f.owner, f.reader).await;
}

#[tokio::test]
async fn typed_actions_emit_exact_bounded_frames() {
    let mut f = fixture(Some(Ok(json!(true)))).await;
    let rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap();
    assert_ne!(rpc.sidecar_incarnation.to_string(), "python-generation");
    assert_eq!(rpc.acquire(acquire(45_000).unwrap()).await.unwrap()
        .into_unverified_value(), json!(true));
    rpc.finish(ResidentAdmission::Resume).await.unwrap().into_unverified_value();
    rpc.finish(ResidentAdmission::Fenced).await.unwrap().into_unverified_value();
    rpc.discard(ResidentDiscard::NotStarted).await.unwrap().into_unverified_value();
    rpc.discard(ResidentDiscard::Unknown).await.unwrap().into_unverified_value();
    let expected = [
        json!({"action": "acquire", "project": "project", "path": "/fixture",
            "wrapper_url": "https://fixture.invalid/wrapper.mjs", "wrapper_export": "b",
            "cookie_header": "session=synthetic-only", "budget_ms": 45_000}),
        json!({"action": "finish", "native_state": "settled", "admission": "resume"}),
        json!({"action": "finish", "native_state": "settled", "admission": "fenced"}),
        json!({"action": "discard", "native_state": "not_started"}),
        json!({"action": "discard", "native_state": "unknown"}),
    ];
    for (index, extra) in expected.into_iter().enumerate() {
        assert_eq!(f.frames.try_recv().unwrap(), expected_frame(index as u64 + 1, extra));
    }
    assert!(f.frames.try_recv().is_err());
    assert!(f.owner.routes.lock().await.is_empty());
    close(&f.owner, f.reader).await;
}

#[tokio::test]
async fn missing_sidecar_cannot_be_bound_or_started() {
    let root = std::env::temp_dir().join(format!("uncreated-resident-{}", Uuid::new_v4()));
    let mut bridge = SidecarBridge::new(&root);
    bridge.configure_resident_controls(ApplicationGeneration(37)).unwrap();
    let owner = Arc::new(bridge);
    assert!(matches!(ResidentRpc::bind(&owner, identity()).await,
        Err(ResidentError::Unconfirmed)));
    assert!(owner.inner.lock().await.is_none());
    assert!(owner.routes.lock().await.is_empty());
    assert!(!root.exists());
}

#[tokio::test]
async fn removed_sidecar_refuses_all_actions_without_registering_or_starting() {
    let mut f = fixture(Some(Ok(json!(true)))).await;
    let rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap();
    let sidecar = f.owner.inner.lock().await.take();
    assert!(matches!(rpc.acquire(acquire(45_000).unwrap()).await,
        Err(ResidentError::Unconfirmed)));
    assert!(matches!(rpc.finish(ResidentAdmission::Resume).await,
        Err(ResidentError::Unconfirmed)));
    assert!(matches!(rpc.discard(ResidentDiscard::Unknown).await,
        Err(ResidentError::Unconfirmed)));
    assert!(f.owner.inner.lock().await.is_none());
    assert!(f.owner.routes.lock().await.is_empty());
    *f.owner.inner.lock().await = sidecar;
    assert_no_command(&mut f).await;
    close(&f.owner, f.reader).await;
}

#[tokio::test]
async fn replacement_while_waiting_for_inner_refuses_before_reply_registration() {
    let mut f = fixture(Some(Ok(json!(true)))).await;
    let rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap();
    let mut inner = f.owner.inner.lock().await;
    let mut call = Box::pin(rpc.acquire(acquire(45_000).unwrap()));
    tokio::select! {
        biased;
        _ = &mut call => panic!("RPC escaped the lifecycle lock"),
        _ = std::future::ready(()) => {}
    }
    inner.as_mut().unwrap().incarnation = Uuid::new_v4();
    drop(inner);
    assert!(matches!(call.await, Err(ResidentError::Unconfirmed)));
    assert!(matches!(rpc.finish(ResidentAdmission::Fenced).await,
        Err(ResidentError::Unconfirmed)));
    assert!(matches!(rpc.discard(ResidentDiscard::NotStarted).await,
        Err(ResidentError::Unconfirmed)));
    assert_no_command(&mut f).await;
    close(&f.owner, f.reader).await;
}

#[tokio::test]
async fn missing_arc_owner_emits_no_command_even_if_pipe_is_still_alive() {
    let f = fixture(Some(Ok(json!(true)))).await;
    let rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap();
    let owner = Arc::try_unwrap(f.owner).unwrap_or_else(|_| panic!("owner was retained"));
    assert!(matches!(rpc.acquire(acquire(45_000).unwrap()).await,
        Err(ResidentError::OwnerUnavailable)));
    assert!(matches!(rpc.finish(ResidentAdmission::Fenced).await,
        Err(ResidentError::OwnerUnavailable)));
    assert!(matches!(rpc.discard(ResidentDiscard::Unknown).await,
        Err(ResidentError::OwnerUnavailable)));
    let mut f = Fixture { owner: Arc::new(owner), ..f };
    assert_no_command(&mut f).await;
    close(&f.owner, f.reader).await;
}

#[tokio::test]
async fn unhealthy_and_exited_sidecars_are_never_used_or_restarted() {
    let mut f = fixture(Some(Ok(json!(true)))).await;
    let rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap();
    {
        let mut inner = f.owner.inner.lock().await;
        let host = inner.as_mut().unwrap();
        host.healthy.store(false, Ordering::Release);
    }
    assert!(matches!(ResidentRpc::bind(&f.owner, identity()).await,
        Err(ResidentError::Unconfirmed)));
    assert!(matches!(rpc.finish(ResidentAdmission::Resume).await,
        Err(ResidentError::Unconfirmed)));
    {
        let mut inner = f.owner.inner.lock().await;
        let host = inner.as_mut().unwrap();
        host.child.kill().await.unwrap();
        host.child.wait().await.unwrap();
        host.healthy.store(true, Ordering::Release);
    }
    assert!(matches!(ResidentRpc::bind(&f.owner, identity()).await,
        Err(ResidentError::Unconfirmed)));
    assert!(matches!(rpc.acquire(acquire(45_000).unwrap()).await,
        Err(ResidentError::Unconfirmed)));
    assert_no_command(&mut f).await;
    tokio::time::timeout(Duration::from_secs(2), f.reader).await.unwrap().unwrap();
}

#[tokio::test]
async fn dropped_waiter_only_requests_cancel_and_never_claims_settlement() {
    let mut f = fixture(None).await;
    let rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap();
    let mut waiter = Box::pin(rpc.acquire(acquire(45_000).unwrap()));
    let frame = tokio::select! {
        _ = &mut waiter => panic!("silent peer supplied no reply"),
        frame = f.frames.recv() => frame.unwrap(),
        _ = tokio::time::sleep(Duration::from_secs(2)) => panic!("no request frame"),
    };
    let id = frame["id"].as_u64().unwrap();
    let pending = f.owner.inner.lock().await.as_ref().unwrap().pending.clone();
    assert!(pending.lock().unwrap().contains_key(&id));
    drop(waiter);
    assert!(pending.lock().unwrap().is_empty());
    assert_eq!(f.cancellations.try_recv().unwrap(), id);
    drop(rpc);
    assert!(f.frames.try_recv().is_err(), "drop must not send finish/discard");
    close(&f.owner, f.reader).await;
}

#[tokio::test]
async fn ordinary_rpc_keeps_default_framing_reply_and_error_behavior() {
    let mut f = fixture_with_generation(Some(Ok(json!({"fixture": true}))), None).await;
    for (index, start_missing) in [true, false].into_iter().enumerate() {
        let reply = f.owner.call_with_policy("ordinary", json!({"id": 77,
            "cmd": "ignored", "body": "synthetic"}), CONTROL_TIMEOUT, start_missing)
            .await.unwrap();
        assert_eq!(reply, json!({"fixture": true}));
        assert_eq!(f.frames.try_recv().unwrap(), json!({"id": index + 1,
            "cmd": "ordinary", "body": "synthetic"}));
    }
    close(&f.owner, f.reader).await;
    let f = fixture(Some(Err("synthetic-remote-error".into()))).await;
    assert_eq!(f.owner.call("ordinary", json!({})).await.err().unwrap(),
        "synthetic-remote-error");
    let rpc = ResidentRpc::bind(&f.owner, identity()).await.unwrap();
    assert_eq!(rpc.finish(ResidentAdmission::Fenced).await.err().unwrap(),
        ResidentError::Unconfirmed);
    close(&f.owner, f.reader).await;
}
