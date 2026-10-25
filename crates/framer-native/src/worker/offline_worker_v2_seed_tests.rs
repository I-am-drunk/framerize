use super::*;

const SDK_SEED: &str = "00112233445546778899aabbccddeeff";

fn isolated_seed_minter(scope: &AccountScope, worker: Arc<V2Worker>,
    device: &str, seed: &str, lost_reply: bool) -> (Arc<Bridge>, Arc<OfflineWorkerMinter>) {
    let mut bridge = Bridge::new(scope, lost_reply);
    let owned = Arc::get_mut(&mut bridge).unwrap();
    Arc::get_mut(&mut owned.grant).unwrap().device = device.into();
    let binding = V2SeedBinding::new(bridge.grant.clone(),
        SavedDevice::new(seed.into()).unwrap());
    let minter = Arc::new(OfflineWorkerMinter::with_v2_seed(scope.clone(), worker,
        bridge.clone(), bridge.grant.clone(), binding).unwrap());
    (bridge, minter)
}

fn recording_fixture() -> (Fixture, Arc<V2Worker>) {
    let source = include_str!("../../../../reference/fixtures/offline_v2_worker_fake.mjs");
    let marker = "fs.closeSync(3);";
    assert_eq!(source.matches(marker).count(), 1);
    let capture = concat!("fs.closeSync(3);\n",
        "fs.writeFileSync(new URL('./observed-device', import.meta.url), ",
        "input.device.id, { mode: 0o600, flag: 'wx' });");
    let source = source.replacen(marker, capture, 1);
    fixture_with_operation(source.as_bytes(),
        "2741224a6e33a3d7f35eccc1d92769c45badf6689516c3f7c7b0593dbff9eaa9")
}

async fn shared_worker_seed_isolation(lost_reply: bool) {
    const OTHER_SEED: &str = "11223344556647778899aabbccddeeff";
    let (fixture, worker) = recording_fixture();
    let (client, scope_a) = account();
    let scope_b = client.open_account(AccountId(uuid::Uuid::new_v4()), Session {
        active_user: FramerUserId("offline-seed-account-b".into()),
        cookie_header: "session=seed-account-b-fixture".into(),
    }, NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    let (bridge_a, minter_a) = isolated_seed_minter(&scope_a, worker.clone(),
        "http-device-a", SDK_SEED, lost_reply);
    let (bridge_b, minter_b) = isolated_seed_minter(&scope_b, worker,
        "http-device-b", OTHER_SEED, false);
    let mut pending_a = prepare(&client, &scope_a, minter_a.clone(), "success");
    let settled_a = pending_a.completion();
    let mut settled_b = None;
    let mut early_a = None;
    let phase = std::panic::AssertUnwindSafe(async {
        bridge_a.observe(&fixture).await;
        release(&fixture);
        bridge_a.at_acknowledgement().await;
        let child_a = bridge_a.child.lock().unwrap().unwrap();
        let observed_a = fs::read_to_string(fixture.root.join("observed-device")).unwrap();
        early_a = (&mut pending_a).now_or_never();
        let before_b = early_a.is_none() && settled_a.clone().now_or_never().is_none()
            && scope_a.closed().now_or_never().is_none();
        // The bridge has checked A's exact child is gone before these paths are reused.
        for name in ["ready", "release", "observed-device"] {
            fs::remove_file(fixture.root.join(name)).unwrap();
        }
        let pending_b = prepare(&client, &scope_b, minter_b.clone(), "success");
        settled_b = Some(pending_b.completion());
        bridge_b.observe(&fixture).await;
        let child_b = bridge_b.child.lock().unwrap().unwrap();
        let separate_child = child_a != child_b && child_start(child_a.0) != Some(child_a.1);
        release(&fixture);
        bridge_b.at_acknowledgement().await;
        let observed_b = fs::read_to_string(fixture.root.join("observed-device")).unwrap();
        bridge_b.acknowledge.add_permits(1);
        let result_b = tokio::time::timeout(Duration::from_secs(3), pending_b).await.unwrap();
        if early_a.is_none() { early_a = (&mut pending_a).now_or_never(); }
        let after_b = early_a.is_none() && settled_a.clone().now_or_never().is_none()
            && !bridge_a.replied.load(Ordering::SeqCst)
            && scope_a.closed().now_or_never().is_none();
        (observed_a, observed_b, result_b, before_b, after_b, separate_child)
    }).catch_unwind().await;
    // Drain both owners before asserting phase facts.
    if !lost_reply || phase.is_err() { pending_a.cancellation().cancel(); }
    bridge_a.acknowledge.add_permits(1);
    bridge_b.acknowledge.add_permits(1);
    let result_a = match early_a {
        Some(value) => Ok(value),
        None => tokio::time::timeout(Duration::from_secs(3), pending_a).await,
    };
    let cleanup_a = tokio::time::timeout(Duration::from_secs(3), settled_a).await;
    let cleanup_b = match settled_b {
        Some(value) => Some(tokio::time::timeout(Duration::from_secs(3), value).await),
        None => None,
    };
    let expected_user = scope_b.active_user().clone();
    let next_b = client.prepare_owned(&scope_b,
        tokio::time::Instant::now() + Duration::from_secs(5), move |session, _| async move {
            PreparationResult::finished(Ok(session.active_user == expected_user
                && session.cookie_header == "session=seed-account-b-fixture"))
        });
    let next_b = match next_b {
        Ok(pending) => {
            let cleanup = pending.completion();
            let result = tokio::time::timeout(Duration::from_secs(3), pending).await;
            Some((result, tokio::time::timeout(Duration::from_secs(3), cleanup).await))
        }
        Err(_) => None,
    };
    let closed_a = client.close_account(&scope_a).unwrap();
    let closed_b = client.close_account(&scope_b).unwrap();
    let (closed_a, closed_b) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(3), closed_a),
        tokio::time::timeout(Duration::from_secs(3), closed_b));
    let (observed_a, observed_b, result_b, before_b, after_b, separate_child) =
        phase.expect("shared worker phase must finish after retaining both cleanup owners");
    assert!(before_b && after_b, "A must retain its acknowledgement while B completes");
    assert!(separate_child, "B may start only after A's exact child has gone");
    assert_eq!(observed_a, SDK_SEED);
    assert_eq!(observed_b, OTHER_SEED);
    assert_eq!(result_b.unwrap().token.0, "fake-fixture-token");
    assert!(matches!(cleanup_b, Some(Ok(Ok(())))));
    assert!(matches!(next_b, Some((Ok(Ok(true)), Ok(Ok(()))))));
    assert_eq!(closed_b.unwrap(), Ok(()));
    assert!(minter_a.scope().same_generation(&scope_a));
    assert!(minter_b.scope().same_generation(&scope_b));
    assert!(!scope_a.same_generation(&scope_b));
    assert_eq!(minter_a.saved_device(), Some("http-device-a"));
    assert_eq!(minter_b.saved_device(), Some("http-device-b"));
    assert_eq!(fs::read_to_string(fixture.root.join("starts")).unwrap(), "1\n1\n");
    if lost_reply {
        assert!(matches!(result_a.unwrap(), Err(PreparationFailure::CleanupUnconfirmed)));
        assert_eq!(cleanup_a.unwrap(), Err(CloseError::PreparationUnconfirmed));
        assert_eq!(closed_a.unwrap(), Err(CloseError::PreparationUnconfirmed));
        assert_fenced(&client, &scope_a);
    } else {
        assert!(matches!(result_a.unwrap(), Err(PreparationFailure::Cancelled)));
        assert_eq!(cleanup_a.unwrap(), Ok(()));
        assert_eq!(closed_a.unwrap(), Ok(()));
    }
    for bridge in [&bridge_a, &bridge_b] {
        assert_eq!(bridge.admitted.load(Ordering::SeqCst), 1);
        assert_eq!(bridge.finished.load(Ordering::SeqCst), 1);
        assert!(bridge.replied.load(Ordering::SeqCst));
        let (pid, start) = bridge.child.lock().unwrap().unwrap();
        assert_ne!(child_start(pid), Some(start));
    }
}

#[tokio::test]
async fn v2_shared_worker_cancelled_account_keeps_other_seed_and_admission() {
    shared_worker_seed_isolation(false).await;
}

#[tokio::test]
async fn v2_shared_worker_lost_acknowledgement_fences_only_its_account() {
    shared_worker_seed_isolation(true).await;
}

#[tokio::test]
async fn v2_seed_is_sent_while_request_device_stays_bound() {
    let (fixture, worker) = recording_fixture();
    let (client, scope) = account();
    let bridge = Bridge::new(&scope, false);
    let seed = V2SeedBinding::new(bridge.grant.clone(),
        SavedDevice::new(SDK_SEED.into()).unwrap());
    let minter = Arc::new(OfflineWorkerMinter::with_v2_seed(scope.clone(),
        worker, bridge.clone(), bridge.grant.clone(), seed).unwrap());
    let pending = prepare(&client, &scope, minter.clone(), "success");
    let completion = pending.completion();
    bridge.observe(&fixture).await;
    release(&fixture);
    bridge.acknowledge.add_permits(1);
    let result = pending.await;
    tokio::time::timeout(Duration::from_secs(3), completion).await.unwrap().unwrap();
    close(&client, &scope).await;
    assert_eq!(result.unwrap().token.0, "fake-fixture-token");
    assert_eq!(minter.saved_device(), Some(bridge.grant.device.as_str()));
    assert_ne!(minter.saved_device(), Some(SDK_SEED));
    assert_eq!(fs::read_to_string(fixture.root.join("observed-device")).unwrap(), SDK_SEED);
    assert_eq!(fs::read_to_string(fixture.root.join("starts")).unwrap(), "1\n");
    assert_eq!(bridge.admitted.load(Ordering::SeqCst), 1);
    assert_eq!(bridge.finished.load(Ordering::SeqCst), 1);
    assert!(bridge.replied.load(Ordering::SeqCst));
}

#[tokio::test]
async fn v2_seed_cannot_move_to_another_native_grant() {
    let (fixture, worker) = fixture();
    let (client, scope) = account();
    let original = Bridge::new(&scope, false);
    let replacement = Bridge::new(&scope, false);
    let seed = V2SeedBinding::new(original.grant.clone(),
        SavedDevice::new(SDK_SEED.into()).unwrap());
    let result = OfflineWorkerMinter::with_v2_seed(scope.clone(), worker,
        replacement.clone(), replacement.grant.clone(), seed);
    close(&client, &scope).await;
    assert!(matches!(result, Err(Failure::Identity)));
    assert!(!fixture.root.join("starts").exists());
    assert_eq!(original.admitted.load(Ordering::SeqCst), 0);
    assert_eq!(replacement.admitted.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn v2_seed_rejects_equal_valued_reallocated_grant() {
    let (fixture, worker) = fixture();
    let (client, scope) = account();
    let original = Bridge::new(&scope, false);
    // Only allocation differs; account, host, lease and device stay identical.
    let replacement = Arc::new(DeviceGrant {
        authority: original.grant.authority,
        account: original.grant.account, host_generation: original.grant.host_generation.clone(),
        lease: original.grant.lease, device: original.grant.device.clone(),
    });
    let seed = V2SeedBinding::new(original.grant.clone(),
        SavedDevice::new(SDK_SEED.into()).unwrap());
    let result = OfflineWorkerMinter::with_v2_seed(scope.clone(), worker,
        original.clone(), replacement, seed);
    close(&client, &scope).await;
    assert!(matches!(result, Err(Failure::Identity)));
    assert!(!fixture.root.join("starts").exists());
    assert_eq!(original.admitted.load(Ordering::SeqCst), 0);
}
