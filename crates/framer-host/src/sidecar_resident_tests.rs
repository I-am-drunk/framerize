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

struct ScriptFixture(std::path::PathBuf);

impl ScriptFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("resident-scripts-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }
}

impl Drop for ScriptFixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn bundled_scripts() -> [(&'static str, &'static str); 4] {
    use super::super::{CONTEXTUAL_AUTHORIZATION, RESIDENT_ACTIVITY,
        RESIDENT_AUTHORIZATION, SCRIPT};
    [("camoufox_host.py", SCRIPT), ("resident_activity.py", RESIDENT_ACTIVITY),
        ("resident_authorization.py", RESIDENT_AUTHORIZATION),
        ("contextual_authorization.py", CONTEXTUAL_AUTHORIZATION)]
}

#[test]
fn embedded_host_refreshes_all_three_resident_support_modules() {
    let root = ScriptFixture::new();
    let bridge = SidecarBridge::new(&root.0);
    let directory = root.0.join(".bin");
    let script = directory.join("camoufox_host.py");
    for _ in 0..2 {
        bridge.extract_embedded_script(&script).unwrap();
        for (name, source) in bundled_scripts() {
            let path = directory.join(name);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
            std::fs::write(path, "# stale fixture\n").unwrap();
        }
    }
    assert!(bridge.inner.try_lock().unwrap().is_none());
}

#[test]
fn explicit_and_observed_overrides_keep_their_own_support_modules() {
    for observed in [false, true] {
        let root = ScriptFixture::new();
        let script = root.0.join("camoufox_host.py");
        for (name, _) in bundled_scripts() {
            std::fs::write(root.0.join(name), "# caller-owned fixture\n").unwrap();
        }
        let mut bridge = SidecarBridge::new(root.0.join("state"));
        if observed {
            bridge.mode = super::super::observer::ProcessMode::Observed {
                script: script.clone(), profile_nonce: Uuid::new_v4(),
            };
        } else {
            bridge = bridge.with_script(&script).unwrap();
        }
        bridge.extract_embedded_script(&script).unwrap();
        for (name, _) in bundled_scripts() {
            assert_eq!(std::fs::read_to_string(root.0.join(name)).unwrap(),
                "# caller-owned fixture\n");
        }
        assert!(!bridge.state_root.exists());
    }
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
