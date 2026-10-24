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

struct Fixture {
    owner: Arc<SidecarBridge>,
    frames: mpsc::UnboundedReceiver<Value>,
    cancellations: mpsc::Receiver<u64>,
    reader: tokio::task::JoinHandle<()>,
}

// Synthetic pipe peer only: no host script, browser, SDK, account or network.
async fn fixture(reply: Option<Result<Value, String>>) -> Fixture {
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
    let owner = Arc::new(SidecarBridge::new("unused-resident-fixture"));
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
    let owner = Arc::new(SidecarBridge::new(&root));
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
    let mut f = fixture(Some(Ok(json!({"fixture": true})))).await;
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
