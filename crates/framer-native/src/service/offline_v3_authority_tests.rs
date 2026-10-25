use super::*;

fn v3_fixture() -> Fixture {
    fixture_with_operation(include_bytes!("../../../../scripts/tests/fixtures/v3_authority_mint.mjs"),
        "2fee605599e5e1340e3b4555225b2fa2ba8985c80ba10cc3222ceb3f03d5188b").0
}

fn observed_host(scope: &AccountScope, lost_reply: bool,
    failure: Option<DeviceFailure>) -> Arc<ProofHost> {
    let mut host = ProofHost::new(scope, lost_reply, failure);
    let inner = Arc::get_mut(&mut Arc::get_mut(&mut host).unwrap().inner).unwrap();
    Arc::get_mut(&mut inner.grant).unwrap().authority = DeviceAuthority::Observation;
    host
}

fn v3_minter(scope: &AccountScope, fixture: &Fixture,
    host: &Arc<ProofHost>) -> Arc<OfflineWorkerMinter> {
    let binding = V2SeedBinding::from_proof(scope, host.inner.grant.clone(),
        &host.session, host.proof()).unwrap();
    Arc::new(OfflineWorkerMinter::with_host_proof(scope.clone(),
        WorkerBackend::V3(fixture.worker.clone()), host.clone(),
        host.inner.grant.clone(), binding).unwrap())
}

#[tokio::test]
async fn v3_missing_or_stale_observation_starts_no_worker() {
    for stale in [false, true] {
        let fixture = v3_fixture();
        let (client, scope) = account();
        let host = observed_host(&scope, false, None);
        let minter = if stale { v3_minter(&scope, &fixture, &host) } else {
            Arc::new(OfflineWorkerMinter::with_device(scope.clone(),
                fixture.worker.clone(), host.clone(), host.inner.grant.clone()).unwrap())
        };
        host.current.store(!stale, Ordering::SeqCst);
        let pending = prepare(&client, &scope, minter, "success");
        let completion = pending.completion();
        let result = pending.await;
        completion.await.unwrap();
        close(&client, &scope).await;
        assert!(matches!(result, Err(PreparationFailure::Rejected)));
        assert!(!fixture.root.join("starts").exists());
        assert_eq!(host.inner.admitted.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn v3_source_proof_failure_never_falls_back_to_generic_admission() {
    use crate::service::source::OfflineSource;
    use crate::authorization::source::SourceBuilder;
    for failure in [DeviceFailure::Rejected, DeviceFailure::Unconfirmed] {
        let fixture = v3_fixture();
        let client = NativeAccountClient::new().unwrap();
        let proxy = "http://127.0.0.1:9";
        let scope = client.open_account(AccountId(uuid::Uuid::new_v4()), Session {
            active_user: FramerUserId("v3-proof-fixture".into()),
            cookie_header: "session=fixture-only".into(),
        }, NativeHttpLimits::default(), NativeEgress::Proxy(proxy)).unwrap();
        let host = observed_host(&scope, false, Some(failure));
        let target = || CredentialTarget {
            project: ProjectId("success".into()), path: "/ai/v3/responses/",
        };
        let mint_target = OfflineMintTarget::new(target(),
            "https://framer.com/projects/success".into()).unwrap();
        let source = OfflineSource::new(fixture.worker.clone(), mint_target, host.clone())
            .build_with_device(scope.clone(), Some(proxy.into()),
                Some(host.inner.grant.clone())).unwrap();
        let selected = scope.clone();
        let pending = client.prepare_owned(&scope,
            tokio::time::Instant::now() + Duration::from_secs(3),
            move |session, control| async move {
                source.issue_for_target(&selected, &session, &target(), &control).await
            }).unwrap();
        let completion = pending.completion();
        let result = pending.await;
        let cleanup = completion.await;
        if failure == DeviceFailure::Rejected {
            assert!(matches!(result, Err(PreparationFailure::Rejected)));
            assert_eq!(cleanup, Ok(()));
            close(&client, &scope).await;
        } else {
            assert!(matches!(result, Err(PreparationFailure::CleanupUnconfirmed)));
            assert_eq!(cleanup, Err(CloseError::PreparationUnconfirmed));
            assert_eq!(tokio::time::timeout(Duration::from_secs(3), scope.closed()).await.unwrap(),
                Err(CloseError::PreparationUnconfirmed));
            assert_fenced(&client, &scope);
        }
        assert_eq!(host.transfers.load(Ordering::SeqCst), 1);
        assert_eq!(host.inner.admitted.load(Ordering::SeqCst), 0);
        assert!(!fixture.root.join("starts").exists());
    }
}

async fn v3_completion_case(mode: u8) {
    let fixture = v3_fixture();
    let (client, scope) = account();
    let host = observed_host(&scope, mode == 3, None);
    let minter = v3_minter(&scope, &fixture, &host);
    let mut pending = prepare(&client, &scope, minter, "success");
    let completion = pending.completion();
    host.inner.observe(&fixture).await;
    if mode == 2 { pending.cancellation().cancel(); } else { release(&fixture); }
    host.inner.at_acknowledgement().await;
    if mode == 1 { host.current.store(false, Ordering::SeqCst); }
    let early = (&mut pending).now_or_never();
    let held = early.is_none() && completion.clone().now_or_never().is_none();
    host.inner.acknowledge.add_permits(1);
    let result = match early {
        Some(value) => value,
        None => tokio::time::timeout(Duration::from_secs(3), pending).await.unwrap(),
    };
    let cleanup = tokio::time::timeout(Duration::from_secs(3), completion).await.unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(3),
        client.close_account(&scope).unwrap()).await.unwrap();
    assert!(held, "V3 must retain the proof-aware host acknowledgement after child join");
    assert_eq!(host.inner.admitted.load(Ordering::SeqCst), 1);
    assert_eq!(host.inner.finished.load(Ordering::SeqCst), 1);
    let (pid, start) = host.inner.child.lock().unwrap().unwrap();
    assert_ne!(child_start(pid), Some(start));
    if mode == 0 {
        assert!(result.unwrap().token.0.starts_with("cNz8rPYH|"));
        assert_eq!(cleanup, Ok(()));
        assert_eq!(closed, Ok(()));
    } else { check_completion(mode, result, cleanup, closed); }
}

#[tokio::test]
async fn v3_observed_mint_preserves_saved_device_and_joined_proof_completion() {
    for mode in 0..4 { v3_completion_case(mode).await; }
}

#[tokio::test]
async fn v3_observation_proof_cannot_move_between_account_scope_owners() {
    let fixture = v3_fixture();
    let (client, scope) = account();
    let host = observed_host(&scope, false, None);
    let other = NativeAccountClient::new().unwrap();
    let other_scope = other.open_account(scope.account(), host.session.clone(),
        NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    let binding = V2SeedBinding::from_proof(&scope, host.inner.grant.clone(),
        &host.session, host.proof()).unwrap();
    let result = OfflineWorkerMinter::with_host_proof(other_scope.clone(),
        WorkerBackend::V3(fixture.worker.clone()), host.clone(),
        host.inner.grant.clone(), binding);
    close(&client, &scope).await;
    close(&other, &other_scope).await;
    assert!(matches!(result, Err(Failure::Identity)));
    assert!(!fixture.root.join("starts").exists());
    assert_eq!(host.inner.admitted.load(Ordering::SeqCst), 0);
}
