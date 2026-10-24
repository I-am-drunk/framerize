use super::*;
use framer_core::native_device::NativeSdkSeed;
use serde_json::{json, Value};

const SEED: &str = "00112233445546778899aabbccddeeff";
const SESSION_ID: &str = "70000000-0000-4000-8000-000000000001";
const OBSERVATION_ID: &str = "70000000-0000-4000-8000-000000000002";

fn session(scope: &AccountScope) -> Session {
    Session { active_user: scope.active_user().clone(), cookie_header: "session=fixture-only".into() }
}

fn wire(grant: &DeviceGrant, session: &Session) -> Value {
    json!({"account": grant.account.0, "generation": grant.host_generation,
        "lease": grant.lease, "account_incarnation": 1, "user": session.active_user.0,
        "session_id": SESSION_ID, "observation_id": OBSERVATION_ID, "seed": SEED})
}

struct ProofHost {
    inner: Arc<Bridge>,
    session: Session,
    current: AtomicBool,
    transfer_failure: Option<DeviceFailure>,
    transfers: AtomicUsize,
    hold_transfer: AtomicBool,
    transfer_arrived: Semaphore,
    transfer_release: Semaphore,
}

impl ProofHost {
    fn new(scope: &AccountScope, lost_reply: bool, failure: Option<DeviceFailure>) -> Arc<Self> {
        let mut inner = Bridge::new(scope, lost_reply);
        Arc::get_mut(&mut Arc::get_mut(&mut inner).unwrap().grant).unwrap().host_generation =
            "0123456789abcdef0123456789abcdef".into();
        Arc::new(Self { inner, session: session(scope), current: AtomicBool::new(true),
            transfer_failure: failure, transfers: AtomicUsize::new(0),
            hold_transfer: AtomicBool::new(false), transfer_arrived: Semaphore::new(0),
            transfer_release: Semaphore::new(0) })
    }

    fn proof(&self) -> NativeSdkSeed {
        NativeSdkSeed::from_wire(self.inner.grant.clone(), &self.session,
            wire(&self.inner.grant, &self.session)).unwrap()
    }

    fn verify(&self, proof: &NativeSdkSeed) {
        assert!(proof.matches(&self.inner.grant, &self.session));
        assert_eq!(proof.account_incarnation(), 1);
        assert_eq!(proof.session_id().to_string(), SESSION_ID);
        assert_eq!(proof.observation_id().to_string(), OBSERVATION_ID);
        assert_eq!(proof.seed(), SEED);
    }
}

#[async_trait::async_trait]
impl BrowserBridge for ProofHost {
    async fn transfer_native_sdk_seed(&self, grant: Arc<DeviceGrant>, session: &Session)
        -> Result<NativeSdkSeed, DeviceFailure> {
        assert!(Arc::ptr_eq(&grant, &self.inner.grant));
        assert_eq!(session.active_user, self.session.active_user);
        assert_eq!(session.cookie_header, self.session.cookie_header);
        self.transfers.fetch_add(1, Ordering::SeqCst);
        if self.hold_transfer.load(Ordering::SeqCst) {
            self.transfer_arrived.add_permits(1);
            self.transfer_release.acquire().await.unwrap().forget();
        }
        if let Some(failure) = self.transfer_failure { return Err(failure); }
        Ok(self.proof())
    }

    async fn admit_native_sdk_mint(&self, proof: &NativeSdkSeed, budget_ms: u64)
        -> Result<u64, DeviceFailure> {
        self.verify(proof);
        if !self.current.load(Ordering::SeqCst) { return Err(DeviceFailure::Rejected); }
        self.inner.admit_native_mint(proof.grant(), budget_ms).await
    }

    async fn finish_native_sdk_mint(&self, proof: &NativeSdkSeed,
        sequence: u64, joined: bool) -> Result<bool, DeviceFailure> {
        self.verify(proof);
        let ready = self.inner.finish_native_mint(proof.grant(), sequence, joined).await?;
        Ok(ready && self.current.load(Ordering::SeqCst))
    }

    async fn castle_token(&self, account: &AccountId) -> Result<CastleToken, String> {
        self.inner.castle_token(account).await
    }
    async fn auth_begin(&self, account: &AccountId, email: &str) -> Result<(), String> {
        self.inner.auth_begin(account, email).await
    }
    async fn auth_complete(&self, account: &AccountId, code: &str)
        -> Result<(String, Value), String> {
        self.inner.auth_complete(account, code).await
    }
}

fn bound_minter(scope: &AccountScope, worker: Arc<V2Worker>, host: &Arc<ProofHost>)
    -> Arc<OfflineWorkerMinter> {
    let binding = V2SeedBinding::from_proof(scope, host.inner.grant.clone(), &host.session,
        host.proof()).unwrap();
    Arc::new(OfflineWorkerMinter::with_v2_seed(scope.clone(), worker, host.clone(),
        host.inner.grant.clone(), binding).unwrap())
}

fn private_inputs() -> (Arc<DeviceGrant>, Session) {
    (Arc::new(DeviceGrant { account: AccountId(uuid::Uuid::new_v4()),
        host_generation: "0123456789abcdef0123456789abcdef".into(),
        lease: uuid::Uuid::new_v4(), device: "http-device-only".into() }),
        Session { active_user: FramerUserId("proof-user".into()),
            cookie_header: "session=proof-fixture".into() })
}

#[test]
fn sdk_transfer_keeps_exact_seed_session_and_original_grant() {
    let (grant, session) = private_inputs();
    let proof = NativeSdkSeed::from_wire(grant.clone(), &session, wire(&grant, &session)).unwrap();
    assert_eq!(proof.seed(), SEED);
    assert_eq!(proof.grant().device, "http-device-only");
    assert!(proof.matches(&grant, &session));
    let replacement = Arc::new(DeviceGrant { account: grant.account,
        host_generation: grant.host_generation.clone(), lease: grant.lease,
        device: grant.device.clone() });
    assert!(!proof.matches(&replacement, &session));
    let mut changed = session.clone();
    changed.cookie_header.push('x');
    assert!(!proof.matches(&grant, &changed));
    changed = session.clone();
    changed.active_user.0.push('x');
    assert!(!proof.matches(&grant, &changed));
}

#[test]
fn sdk_admission_requires_original_lease_and_both_proof_ids() {
    let (grant, session) = private_inputs();
    let proof = NativeSdkSeed::from_wire(grant.clone(), &session, wire(&grant, &session)).unwrap();
    let reply = json!({"generation": grant.host_generation, "lease": grant.lease,
        "sequence": 73, "session_id": SESSION_ID, "observation_id": OBSERVATION_ID});
    assert_eq!(proof.admission_from_wire(&reply), Ok(73));
    for (key, value) in [("generation", json!("changed")), ("lease", json!(uuid::Uuid::new_v4())),
        ("session_id", json!(uuid::Uuid::new_v4())),
        ("observation_id", json!(uuid::Uuid::new_v4())), ("sequence", json!(0))] {
        let mut changed = reply.clone();
        changed[key] = value;
        assert_eq!(proof.admission_from_wire(&changed), Err(DeviceFailure::Unconfirmed));
    }
    let mut legacy = reply;
    legacy.as_object_mut().unwrap().remove("session_id");
    legacy.as_object_mut().unwrap().remove("observation_id");
    assert_eq!(proof.admission_from_wire(&legacy), Err(DeviceFailure::Unconfirmed));
}

#[test]
fn sdk_finish_distinguishes_stale_proof_from_unconfirmed_cleanup() {
    let (grant, session) = private_inputs();
    let proof = NativeSdkSeed::from_wire(grant.clone(), &session, wire(&grant, &session)).unwrap();
    let mut reply = json!({"confirmed": true, "ready": false,
        "session_id": SESSION_ID, "observation_id": OBSERVATION_ID});
    assert_eq!(proof.completion_from_wire(&reply), Ok(false));
    reply["ready"] = json!(true);
    assert_eq!(proof.completion_from_wire(&reply), Ok(true));
    for (key, value) in [("confirmed", json!(false)), ("ready", Value::Null),
        ("session_id", json!(uuid::Uuid::new_v4())),
        ("observation_id", json!(uuid::Uuid::new_v4()))] {
        let mut changed = reply.clone();
        changed[key] = value;
        assert_eq!(proof.completion_from_wire(&changed), Err(DeviceFailure::Unconfirmed));
    }
    reply.as_object_mut().unwrap().remove("session_id");
    reply.as_object_mut().unwrap().remove("observation_id");
    assert_eq!(proof.completion_from_wire(&reply), Err(DeviceFailure::Unconfirmed));
}

#[tokio::test]
async fn sdk_proof_cannot_move_between_account_scope_owners() {
    let (fixture, worker) = fixture();
    let (client, scope) = account();
    let host = ProofHost::new(&scope, false, None);
    let other = NativeAccountClient::new().unwrap();
    let other_scope = other.open_account(scope.account(), host.session.clone(),
        NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    let seed = V2SeedBinding::from_proof(&scope, host.inner.grant.clone(),
        &host.session, host.proof()).unwrap();
    let result = OfflineWorkerMinter::with_v2_seed(other_scope.clone(), worker,
        host.clone(), host.inner.grant.clone(), seed);
    close(&client, &scope).await;
    close(&other, &other_scope).await;
    assert!(matches!(result, Err(Failure::Identity)));
    assert!(!fixture.root.join("starts").exists());
    assert_eq!(host.inner.admitted.load(Ordering::SeqCst), 0);
}

#[test]
fn sdk_transfer_rejects_foreign_malformed_and_incomplete_wire() {
    let (grant, session) = private_inputs();
    let original = wire(&grant, &session);
    let mut refused = vec![Value::Null, json!([])];
    for key in original.as_object().unwrap().keys() {
        let mut missing = original.clone();
        missing.as_object_mut().unwrap().remove(key);
        refused.push(missing);
    }
    let mut extra = original.clone();
    extra["device"] = json!("http-must-not-be-seed");
    refused.push(extra);
    for (key, value) in [
        ("account", json!(uuid::Uuid::new_v4())),
        ("generation", json!("fedcba9876543210fedcba9876543210")),
        ("lease", json!(uuid::Uuid::new_v4())), ("user", json!("other-user")),
        ("account_incarnation", json!(0)), ("account_incarnation", json!(-1)),
        ("account_incarnation", json!(1.5)), ("session_id", json!(uuid::Uuid::nil())),
        ("observation_id", json!(uuid::Uuid::nil())),
        ("session_id", json!(SESSION_ID.replace('-', ""))),
        ("observation_id", json!("70000000-0000-4000-8000-00000000000A")),
        ("seed", json!(SEED.to_uppercase())), ("seed", json!("http-device-only")),
        ("seed", json!("00112233445566778899aabbccddeeff")),
        ("seed", json!("00112233445546777899aabbccddeeff")),
    ] {
        let mut changed = original.clone();
        changed[key] = value;
        refused.push(changed);
    }
    for value in refused {
        assert!(matches!(NativeSdkSeed::from_wire(grant.clone(), &session, value),
            Err(DeviceFailure::Unconfirmed)));
    }
}

#[tokio::test]
async fn sdk_missing_or_stale_proof_starts_no_worker() {
    for missing in [true, false] {
        let (fixture, worker) = fixture();
        let (client, scope) = account();
        let host = ProofHost::new(&scope, false, None);
        let minter = if missing {
            Arc::new(OfflineWorkerMinter::with_v2_device(scope.clone(), worker,
                host.clone(), host.inner.grant.clone()).unwrap())
        } else {
            let minter = bound_minter(&scope, worker, &host);
            host.current.store(false, Ordering::SeqCst);
            minter
        };
        let pending = prepare(&client, &scope, minter, "success");
        let completion = pending.completion();
        let result = pending.await;
        completion.await.unwrap();
        close(&client, &scope).await;
        assert!(matches!(result, Err(PreparationFailure::Rejected)));
        assert!(!fixture.root.join("starts").exists());
        assert_eq!(host.inner.admitted.load(Ordering::SeqCst), 0);
        assert_eq!(host.inner.finished.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn sdk_source_transfer_failure_preserves_cleanup_without_credential_request() {
    use crate::{offline_source::OfflineSource, one_shot_control::SourceBuilder};
    for failure in [DeviceFailure::Rejected, DeviceFailure::Unconfirmed] {
        let (fixture, worker) = fixture();
        let (client, scope) = account();
        let host = ProofHost::new(&scope, false, Some(failure));
        let target = || CredentialTarget {
            project: ProjectId("success".into()), path: "/ai/v3/responses/",
        };
        let mint_target = OfflineMintTarget::new(target(),
            "https://framer.com/projects/success".into()).unwrap();
        let source = OfflineSource::new_v2(worker, mint_target, host.clone())
            .build_with_device(scope.clone(), None, Some(host.inner.grant.clone())).unwrap();
        let selected = scope.clone();
        let pending = client.prepare_owned(&scope, tokio::time::Instant::now() + Duration::from_secs(3),
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

async fn cancelled_transfer_case(failure: Option<DeviceFailure>, dropped: bool) {
    use crate::{offline_source::OfflineSource, one_shot_control::SourceBuilder};
    let proxy_guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    proxy_guard.set_nonblocking(true).unwrap();
    let proxy = format!("http://{}", proxy_guard.local_addr().unwrap());
    let client = NativeAccountClient::new().unwrap();
    let scope = client.open_account(AccountId(uuid::Uuid::new_v4()), Session {
        active_user: FramerUserId("transfer-cancel-fixture".into()),
        cookie_header: "session=fixture-only".into(),
    }, NativeHttpLimits::default(), NativeEgress::Proxy(&proxy)).unwrap();
    let (fixture, worker) = fixture();
    let host = ProofHost::new(&scope, false, failure);
    host.hold_transfer.store(true, Ordering::SeqCst);
    let target = || CredentialTarget {
        project: ProjectId("success".into()), path: "/ai/v3/responses/",
    };
    let mint_target = OfflineMintTarget::new(target(),
        "https://framer.com/projects/success".into()).unwrap();
    let source = OfflineSource::new_v2(worker, mint_target, host.clone())
        .build_with_device(scope.clone(), Some(proxy), Some(host.inner.grant.clone())).unwrap();
    let selected = scope.clone();
    let mut pending = Some(client.prepare_owned(&scope,
        tokio::time::Instant::now() + Duration::from_secs(5), move |session, control| async move {
            source.issue_for_target(&selected, &session, &target(), &control).await
        }).unwrap());
    let completion = pending.as_ref().unwrap().completion();
    let arrived = match tokio::time::timeout(Duration::from_secs(3),
        host.transfer_arrived.acquire()).await {
        Ok(Ok(permit)) => { permit.forget(); true }
        _ => false,
    };
    if dropped { drop(pending.take()); }
    else { pending.as_ref().unwrap().cancellation().cancel(); }
    let close_request = client.close_account(&scope);
    let close_requested = close_request.is_ok();
    let closed = close_request.unwrap_or_else(|_| scope.closed());
    let held = tokio::time::timeout(Duration::from_millis(100), async {
        tokio::select! {
            _ = completion.clone() => (),
            _ = closed.clone() => (),
        }
    }).await.is_err();
    // Release and drain the original transfer before asserting the observations.
    host.transfer_release.add_permits(1);
    let result = match pending {
        Some(pending) => Some(tokio::time::timeout(Duration::from_secs(3), pending).await.unwrap()),
        None => None,
    };
    let cleanup = tokio::time::timeout(Duration::from_secs(3), completion).await.unwrap();
    let close_result = tokio::time::timeout(Duration::from_secs(3), closed).await.unwrap();
    assert!(arrived && held && close_requested,
        "transfer settlement must retain completion and account close");
    let unknown = failure == Some(DeviceFailure::Unconfirmed);
    if let Some(result) = result {
        if unknown { assert!(matches!(result, Err(PreparationFailure::CleanupUnconfirmed))); }
        else { assert!(matches!(result, Err(PreparationFailure::Cancelled))); }
    }
    let expected = if unknown { Err(CloseError::PreparationUnconfirmed) } else { Ok(()) };
    assert_eq!(cleanup, expected);
    assert_eq!(close_result, expected);
    if unknown { assert_fenced(&client, &scope); }
    assert_eq!(host.transfers.load(Ordering::SeqCst), 1);
    assert_eq!(host.inner.admitted.load(Ordering::SeqCst), 0);
    assert_eq!(host.inner.finished.load(Ordering::SeqCst), 0);
    assert!(!fixture.root.join("starts").exists());
    assert!(matches!(proxy_guard.accept(), Err(error)
        if error.kind() == std::io::ErrorKind::WouldBlock), "no credential connection after cancellation");
}

#[tokio::test]
async fn sdk_cancelled_transfer_retains_settlement_and_unknown_cleanup() {
    for dropped in [false, true] {
        for failure in [None, Some(DeviceFailure::Rejected), Some(DeviceFailure::Unconfirmed)] {
            cancelled_transfer_case(failure, dropped).await;
        }
    }
}

async fn proof_completion_case(mode: u8) {
    let (fixture, worker) = fixture();
    let (client, scope) = account();
    let host = ProofHost::new(&scope, mode == 3, None);
    let minter = bound_minter(&scope, worker, &host);
    let mut pending = prepare(&client, &scope, minter.clone(), "success");
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
    assert!(held, "proof-bound mint must retain the host acknowledgement after join");
    assert_eq!(minter.saved_device(), Some(host.inner.grant.device.as_str()));
    assert_eq!(host.inner.admitted.load(Ordering::SeqCst), 1);
    assert_eq!(host.inner.finished.load(Ordering::SeqCst), 1);
    let (pid, start) = host.inner.child.lock().unwrap().unwrap();
    assert_ne!(child_start(pid), Some(start));
    check_completion(mode, result, cleanup, closed);
}

fn check_completion(mode: u8, result: Result<MintedCastle, PreparationFailure>,
    cleanup: Result<(), CloseError>, closed: Result<(), CloseError>) {
    match mode {
        0 => assert_eq!(result.unwrap().token.0, "fake-fixture-token"),
        1 => assert!(matches!(result, Err(PreparationFailure::Rejected))),
        2 => assert!(matches!(result, Err(PreparationFailure::Cancelled))),
        3 => assert!(matches!(result, Err(PreparationFailure::CleanupUnconfirmed))),
        _ => unreachable!(),
    }
    let expected = if mode == 3 { Err(CloseError::PreparationUnconfirmed) } else { Ok(()) };
    assert_eq!(cleanup, expected);
    assert_eq!(closed, expected);
}

#[tokio::test]
async fn sdk_proof_completion_retains_join_ack_and_stale_refusal() {
    for mode in 0..4 { proof_completion_case(mode).await; }
}
