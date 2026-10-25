use super::*;
use crate::worker::process::SavedDevice;
use crate::worker::process::Specification;
use crate::worker::process::Target;

#[path = "offline_worker_v2_refusal_tests.rs"]
mod host_refusal;

#[path = "offline_worker_v2_seed_tests.rs"]
mod seed_binding;

#[path = "offline_v2_seed_handoff_tests.rs"]
mod seed_handoff;

fn fixture() -> (Fixture, Arc<V2Worker>) {
    fixture_with_operation(
        include_bytes!("../../../../reference/fixtures/offline_v2_worker_fake.mjs"),
        "e3a3e8e7fd928778b8d7f5087b78deec63662d5c0f71c7d521f47997d41bd2ce")
}

fn fixture_with_operation(operation: &[u8], operation_hash: &str)
    -> (Fixture, Arc<V2Worker>) {
    let root = std::env::temp_dir().join(format!("framerize-v2-minter-{}", uuid::Uuid::new_v4()));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    fs::write(root.join("worker.mjs"),
        include_bytes!("../../../../scripts/tests/fixtures/v2_minter_gate.mjs")).unwrap();
    fs::write(root.join("operation.mjs"), operation).unwrap();
    let specification = || Specification {
        node: crate::test_support::node(), entry: root.join("worker.mjs"),
        pins: vec![
            crate::test_support::node_pin(),
            pin(root.join("worker.mjs"),
                "07da7878bf7aec97e57187639911e0aa42911ba51fd2789590e18e1419cbae52"),
            pin(root.join("operation.mjs"), operation_hash),
        ],
    };
    let v2 = Arc::new(V2Worker::new(specification()).unwrap());
    let worker = Arc::new(Worker::new(specification()).unwrap());
    (Fixture { root, worker }, v2)
}

fn minter(scope: &AccountScope, worker: Arc<V2Worker>, bridge: &Arc<Bridge>)
    -> Arc<OfflineWorkerMinter> {
    let seed = V2SeedBinding::new(bridge.grant.clone(),
        SavedDevice::new("00112233445546778899aabbccddeeff".into()).unwrap());
    Arc::new(OfflineWorkerMinter::with_v2_seed(scope.clone(), worker,
        bridge.clone(), bridge.grant.clone(), seed).unwrap())
}

fn release(fixture: &Fixture) { fs::write(fixture.root.join("release"), b"ready").unwrap(); }

fn held_exit_fixture() -> (Fixture, Arc<V2Worker>) {
    let source = include_str!("../../../../reference/fixtures/offline_v2_worker_fake.mjs");
    let marker = "process.exit(";
    assert_eq!(source.matches(marker).count(), 1);
    let gate = concat!(
        "fs.writeFileSync(new URL('./handed-off', import.meta.url), 'ready', ",
        "{ mode: 0o600, flag: 'wx' });\n",
        "while (!fs.existsSync(new URL('./exit-release', import.meta.url))) ",
        "await new Promise(resolve => setTimeout(resolve, 5));\nprocess.exit(");
    let source = source.replacen(marker, gate, 1);
    fixture_with_operation(source.as_bytes(),
        "3d33325d26926dbdeefd7a3b303a0a678aac4ace416b2a6f415c4b7070975f26")
}

async fn at_handoff(fixture: &Fixture) -> bool {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !fixture.root.join("handed-off").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.is_ok()
}

async fn post_handoff_case(mode: &str, cancel: bool) {
    let (fixture, worker) = held_exit_fixture();
    let (client, scope) = account();
    let bridge = Bridge::new(&scope, false);
    let mut pending = prepare(&client, &scope, minter(&scope, worker, &bridge), mode);
    let settled = pending.completion();
    bridge.observe(&fixture).await;
    release(&fixture);
    let handed_off = at_handoff(&fixture).await;
    let early = tokio::time::timeout(Duration::from_millis(100), &mut pending).await;
    let token_waited = early.is_err();
    let finish_waited = bridge.finished.load(Ordering::SeqCst) == 0;
    let join_waited = settled.clone().now_or_never().is_none();
    let (pid, start) = bridge.child.lock().unwrap().unwrap();
    let child_waited = child_start(pid) == Some(start);
    if cancel || !handed_off { pending.cancellation().cancel(); }
    else { fs::write(fixture.root.join("exit-release"), b"ready").unwrap(); }
    let closed = if cancel { Some(client.close_account(&scope).unwrap()) } else { None };
    bridge.at_acknowledgement().await;
    let before_ack = if early.is_err() {
        tokio::time::timeout(Duration::from_millis(100), &mut pending).await.ok()
    } else { None };
    let ack_waited = before_ack.is_none() && settled.clone().now_or_never().is_none()
        && closed.as_ref().map_or(true, |value| value.clone().now_or_never().is_none());
    bridge.acknowledge.add_permits(1);
    let result = match (early, before_ack) {
        (Ok(value), _) | (Err(_), Some(value)) => value,
        (Err(_), None) => tokio::time::timeout(Duration::from_secs(3), pending).await.unwrap(),
    };
    tokio::time::timeout(Duration::from_secs(3), settled).await.unwrap().unwrap();
    if let Some(closed) = closed {
        tokio::time::timeout(Duration::from_secs(3), closed).await.unwrap().unwrap();
    } else { close(&client, &scope).await; }
    // Assert the gates only after draining the same child and account owners.
    assert!(handed_off, "fixture must close both handoff pipes before the hold");
    assert!(child_waited, "the held child must remain alive after handoff");
    assert!(token_waited && finish_waited && join_waited, "handoff cannot substitute for join");
    assert!(ack_waited, "result, completion and account close must retain host acknowledgement");
    assert_ne!(child_start(pid), Some(start));
    if cancel { assert!(matches!(result, Err(PreparationFailure::Cancelled))); }
    else if mode == "failed" {
        assert!(matches!(result, Err(PreparationFailure::WorkerLost)));
    } else { assert_eq!(result.unwrap().token.0, "fake-fixture-token"); }
    assert_eq!(bridge.admitted.load(Ordering::SeqCst), 1);
    assert_eq!(bridge.finished.load(Ordering::SeqCst), 1);
    assert!(bridge.replied.load(Ordering::SeqCst));
    assert_eq!(fs::read_to_string(fixture.root.join("starts")).unwrap(), "1\n");
}

#[tokio::test]
async fn v2_post_handoff_waits_for_exit_and_host_acknowledgement() {
    post_handoff_case("success", false).await;
}

#[tokio::test]
async fn v2_post_handoff_failed_exit_discards_valid_token() {
    post_handoff_case("failed", false).await;
}

#[tokio::test]
async fn v2_post_handoff_cancellation_retains_cleanup() {
    post_handoff_case("success", true).await;
}

#[tokio::test]
async fn cancelled_before_first_result_poll_still_signals_child() {
    let (fixture, worker) = fixture();
    let fixture = Arc::new(fixture);
    let (client, scope) = account();
    let minter = Arc::new(OfflineWorkerMinter {
        scope: scope.clone(), worker: WorkerBackend::V2(worker.clone()),
        device: None, sdk_seed: None, diagnostic_id: None,
    });
    let owned = fixture.clone();
    let (ready, observed) = tokio::sync::oneshot::channel();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut pending = client.prepare_owned(&scope, deadline, move |_, control| async move {
        let target = Target { project_id: "hang".into(), path: "/ai/v3/responses/".into(),
            page_url: "https://framer.com/projects/hang".into() };
        let device = SavedDevice::new("fixture-device".into()).unwrap();
        let run = worker.begin_with_device(target, &device, deadline.into_std()).unwrap();
        let child = owned.started().await;
        // No call to Run::result has happened when this cancellation arrives.
        control.cancel();
        assert!(ready.send((run.completion(), child)).is_ok());
        minter.finish_worker(Ok(run), &control).await
    }).unwrap();
    let settled = pending.completion();
    let (completion, (pid, start)) = tokio::time::timeout(Duration::from_secs(4), observed)
        .await.unwrap().unwrap();
    let prompt = tokio::time::timeout(Duration::from_secs(1), &mut pending).await;
    let promptly_cancelled = matches!(&prompt, Ok(Err(PreparationFailure::Cancelled)));
    // Drain the original owner before asserting, including on the broken path.
    assert_eq!(tokio::time::timeout(Duration::from_secs(11), completion.wait()).await.unwrap(),
        Cleanup::Joined { pid });
    if prompt.is_err() {
        assert!(matches!(pending.await, Err(PreparationFailure::Cancelled)));
    }
    tokio::time::timeout(Duration::from_secs(2), settled).await.unwrap().unwrap();
    assert_ne!(child_start(pid), Some(start));
    close(&client, &scope).await;
    assert!(promptly_cancelled, "an unpolled result must not delay child cancellation");
}

#[tokio::test]
async fn v2_operation_failure_does_not_retry_or_fall_back() {
    let (fixture, worker) = fixture();
    let (client, scope) = account();
    let bridge = Bridge::new(&scope, false);
    let pending = prepare(&client, &scope, minter(&scope, worker, &bridge), "refused");
    bridge.observe(&fixture).await;
    release(&fixture);
    bridge.acknowledge.add_permits(1);
    assert!(matches!(pending.await, Err(PreparationFailure::WorkerLost)));
    assert_eq!(bridge.admitted.load(Ordering::SeqCst), 1);
    assert_eq!(bridge.finished.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read_to_string(fixture.root.join("starts")).unwrap(), "1\n");
    close(&client, &scope).await;
}

#[tokio::test]
async fn v2_without_device_refuses_before_child() {
    let (fixture, worker) = fixture();
    let (client, scope) = account();
    let minter = Arc::new(OfflineWorkerMinter {
        scope: scope.clone(), worker: WorkerBackend::V2(worker),
        device: None, sdk_seed: None, diagnostic_id: None,
    });
    let pending = prepare(&client, &scope, minter, "success");
    assert!(matches!(pending.await, Err(PreparationFailure::Rejected)));
    assert!(!fixture.root.join("starts").exists());
    close(&client, &scope).await;
}

#[tokio::test]
async fn v2_rejects_foreign_device_grant_before_child() {
    let (fixture, worker) = fixture();
    let (client, scope) = account();
    let (other_client, other_scope) = account();
    let bridge = Bridge::new(&scope, false);
    assert!(matches!(OfflineWorkerMinter::with_v2_device(other_scope.clone(), worker,
        bridge.clone(), bridge.grant.clone()), Err(Failure::Identity)));
    assert!(!fixture.root.join("starts").exists());
    close(&client, &scope).await;
    close(&other_client, &other_scope).await;
}

#[tokio::test]
async fn v2_lost_acknowledgement_retains_account_fence() {
    let (fixture, worker) = fixture();
    let (client, scope) = account();
    let bridge = Bridge::new(&scope, true);
    let pending = prepare(&client, &scope, minter(&scope, worker, &bridge), "hang");
    bridge.observe(&fixture).await;
    let settled = pending.completion();
    drop(pending);
    bridge.at_acknowledgement().await;
    held_until_acknowledged(settled.clone(), scope.closed()).await;
    bridge.acknowledge.add_permits(1);
    assert_eq!(tokio::time::timeout(Duration::from_secs(3), settled).await.unwrap(),
        Err(CloseError::PreparationUnconfirmed));
    assert_eq!(tokio::time::timeout(Duration::from_secs(3), scope.closed()).await.unwrap(),
        Err(CloseError::PreparationUnconfirmed));
    assert_fenced(&client, &scope);
}

#[tokio::test]
async fn v2_dropped_waiter_retains_host_acknowledgement() {
    let (fixture, worker) = fixture();
    let (client, scope) = account();
    let bridge = Bridge::new(&scope, false);
    let pending = prepare(&client, &scope, minter(&scope, worker, &bridge), "hang");
    bridge.observe(&fixture).await;
    let settled = pending.completion();
    drop(pending);
    bridge.at_acknowledgement().await;
    let closed = client.close_account(&scope).unwrap();
    held_until_acknowledged(settled.clone(), closed.clone()).await;
    bridge.acknowledge.add_permits(1);
    tokio::time::timeout(Duration::from_secs(3), settled).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(3), closed).await.unwrap().unwrap();
    assert!(bridge.replied.load(Ordering::SeqCst));
}

#[tokio::test]
async fn default_v3_refuses_v2_handoff() {
    let (fixture, _) = fixture();
    let (client, scope) = account();
    let bridge = Bridge::new(&scope, false);
    let pending = prepare(&client, &scope, bridge.minter(&scope, &fixture), "success");
    bridge.observe(&fixture).await;
    release(&fixture);
    bridge.acknowledge.add_permits(1);
    assert!(matches!(pending.await, Err(PreparationFailure::Rejected)));
    close(&client, &scope).await;
}

#[tokio::test]
async fn v2_join_preserves_scope_device_and_origin() {
    let (fixture, worker) = fixture();
    let (client, scope) = account();
    let bridge = Bridge::new(&scope, false);
    let minter = minter(&scope, worker, &bridge);
    assert!(minter.scope().same_generation(&scope));
    assert_eq!(minter.saved_device(), Some(bridge.grant.device.as_str()));
    let before = Instant::now();
    let pending = prepare(&client, &scope, minter, "success");
    bridge.observe(&fixture).await;
    assert_eq!(bridge.admitted.load(Ordering::SeqCst), 1);
    release(&fixture);
    bridge.acknowledge.add_permits(1);
    let result = pending.await.unwrap();
    assert_eq!(result.token.0, "fake-fixture-token");
    assert!(result.received_at >= before);
    assert!(result.received_at.elapsed() >= Duration::from_millis(200));
    assert_eq!(bridge.finished.load(Ordering::SeqCst), 1);
    assert!(bridge.replied.load(Ordering::SeqCst));
    close(&client, &scope).await;
    assert_eq!(fs::read_to_string(fixture.root.join("starts")).unwrap(), "1\n");
}
