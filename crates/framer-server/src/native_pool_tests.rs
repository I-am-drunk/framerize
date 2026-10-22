//! Native dispatch through the real pool and router, with in-memory work.
use super::*;
use framer_core::native_dispatch as n;
use futures::FutureExt;
use tower::ServiceExt;
use tokio::sync::OwnedSemaphorePermit;

struct Probe {
    timeout: Duration,
    preparations: AtomicUsize,
    prepare_gate: Semaphore,
    cleanup_gates: [Arc<Semaphore>; 2],
    launched: Arc<AtomicUsize>,
    cleaned: Arc<AtomicUsize>,
    fail_result: AtomicBool,
    member: AtomicBool,
}
struct Call {
    admission: n::PoolAdmission,
    permit: OwnedSemaphorePermit,
    cleanup_gate: Arc<Semaphore>,
    cleaned: Arc<AtomicUsize>,
    fail_result: bool,
    launched: Arc<AtomicUsize>,
}
struct NoCancel;
impl n::Cancellation for NoCancel { fn cancel(&self) {} }

#[tokio::test]
async fn completed_native_cleanup_preserves_a_legacy_lifecycle_fence() {
    let (state, ports, probe) = native_fixture(Duration::from_secs(3));
    let before = records(&ports);
    probe.prepare_gate.add_permits(1);
    let task = tokio::spawn(request(state.clone(), Some(1)));
    until(|| probe.launched.load(Ordering::SeqCst) == 1).await;
    let legacy = super::super::RequestResidency::new(lease_account(&state, id(1)).await.unwrap());
    let error = "account browser cleanup is unconfirmed; retry after cleanup";
    assert!(legacy.observe_result(id(1), Err::<(), _>(error.to_string())).is_err());
    assert_eq!(slot(&state, id(1)).users, 2);
    drop(legacy);
    assert_eq!(slot(&state, id(1)).users, 1);
    probe.cleanup_gates[0].add_permits(1);
    consume(task, 200).await;
    assert_eq!(probe.cleaned.load(Ordering::SeqCst), 1);
    assert_eq!(slot(&state, id(1)).users, 0);
    assert!(slot(&state, id(1)).uncertain);
    assert_eq!(records(&ports), before);
    assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1)), ("status", id(1))]);
}

#[tokio::test]
async fn delayed_status_cannot_release_abandoned_native_pool_admission() {
    let (state, ports) = fixture(1, 2);
    let state = crate::operation_ownership::with_owned_operations(state, 32).unwrap();
    let before = records(&ports);
    let resident = lease_account(&state, id(1)).await.unwrap();
    let owner = state.operation_ownership.as_ref().unwrap().native_cleanup().unwrap();
    let mut admission = n::PoolAdmission::new(resident, Some(owner));
    admission.start();
    ports.hold_status.store(true, Ordering::SeqCst);
    let mut waiting = Box::pin(lease_account(&state, id(1)));
    assert!(futures::poll!(waiting.as_mut()).is_pending());
    assert_eq!(ports.status_captured.load(Ordering::SeqCst), 1);
    drop(admission);
    assert_eq!(slot(&state, id(1)).users, 2);
    ports.status_gate.add_permits(1);
    assert!(matches!(waiting.await, Err(ApiError::PoolBusy(_))));
    assert_eq!(slot(&state, id(1)).users, 1);
    assert!(slot(&state, id(1)).uncertain);
    assert!(matches!(parked(&state, 1).await, Err(ApiError::PoolBusy(_))));
    let Json(view) = crate::pool_shape(State(state.clone())).await.unwrap();
    assert_eq!(view["available_now"], 0);
    assert_eq!(records(&ports), before);
    assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1)), ("status", id(1))]);
    assert!(matches!(state.operation_ownership.as_ref().unwrap().seal(),
        Err(framer_core::operation_gate::GateError::Unconfirmed)));
}

#[tokio::test]
async fn native_model_wait_rejects_new_lifecycle_uncertainty_before_preparation() {
    let (state, ports, probe) = native_fixture(Duration::from_secs(3));
    let before = records(&ports);
    let (held, task) = block_model(&state, Some(1)).await;
    let observer = lease_account(&state, id(1)).await.unwrap();
    observer.mark_unconfirmed().unwrap();
    drop(observer);
    drop(held);
    consume(task, 503).await;
    assert_eq!(probe.preparations.load(Ordering::SeqCst), 0);
    assert_eq!(probe.launched.load(Ordering::SeqCst), 0);
    assert_eq!(slot(&state, id(1)).users, 0);
    assert!(slot(&state, id(1)).uncertain);
    assert_eq!(records(&ports), before);
    assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1)), ("status", id(1))]);
}

async fn consume(task: tokio::task::JoinHandle<axum::response::Response>, expected: u16) {
    let response = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
    assert_eq!(response.status().as_u16(), expected);
    to_bytes(response.into_body(), 4096).await.unwrap();
}

async fn pending_cleanup(fail_result: bool) -> (AppState, Arc<Ports>, Arc<Probe>) {
    let (state, ports, probe) = native_fixture(Duration::from_millis(250));
    probe.fail_result.store(fail_result, Ordering::SeqCst);
    probe.prepare_gate.add_permits(1);
    let task = tokio::spawn(request(state.clone(), Some(1)));
    until(|| probe.launched.load(Ordering::SeqCst) == 1).await;
    consume(task, if fail_result { 502 } else { 504 }).await;
    assert_eq!(slot(&state, id(1)).users, 1);
    reject_pool_mutations(&state, &ports).await;
    (state, ports, probe)
}

#[tokio::test]
async fn cancelling_native_http_waiter_keeps_admitted_work_and_pool_ownership() {
    let (state, ports, probe) = native_fixture(Duration::from_secs(5));
    let task = tokio::spawn(request(state.clone(), Some(1)));
    until(|| probe.preparations.load(Ordering::SeqCst) == 1).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    reject_pool_mutations(&state, &ports).await;
    probe.prepare_gate.add_permits(1);
    until(|| probe.launched.load(Ordering::SeqCst) == 1).await;
    assert_eq!(slot(&state, id(1)).users, 1);
    probe.cleanup_gates[0].add_permits(1);
    until(|| probe.cleaned.load(Ordering::SeqCst) == 1).await;
    assert_eq!(slot(&state, id(1)).users, 0);
    assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1)), ("status", id(2))]);
}

#[tokio::test]
async fn native_error_and_deadline_keep_cleanup_owned_while_another_account_progresses() {
    for fail_result in [true, false] {
        let (state, ports, probe) = pending_cleanup(fail_result).await;
        probe.fail_result.store(false, Ordering::SeqCst);
        probe.prepare_gate.add_permits(1);
        probe.cleanup_gates[1].add_permits(1);
        consume(tokio::spawn(request(state.clone(), Some(2))), 200).await;
        assert_eq!(slot(&state, id(1)).users, 1);
        assert_eq!(slot(&state, id(2)).users, 0);
        let ownership = state.operation_ownership.as_ref().unwrap();
        let mut ticket = ownership.seal().unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(5), ticket.wait_application()).await.is_err());
        probe.cleanup_gates[0].add_permits(1);
        let proof = tokio::time::timeout(Duration::from_secs(2), ticket.wait_application()).await.unwrap().unwrap();
        ownership.check_application_drained(&proof).unwrap();
        drop(proof);
        assert_eq!(probe.cleaned.load(Ordering::SeqCst), 2);
        assert_eq!(slot(&state, id(1)).users, 0);
        assert!(tokio::time::timeout(Duration::from_millis(5), ticket.wait()).await.is_err());
        assert_eq!(*ports.events.lock().unwrap(),
            vec![("status", id(1)), ("status", id(2)), ("status", id(2))]);
    }
}

async fn admin(state: &AppState, path: &str, body: Value) -> u16 {
    let req = axum::http::Request::builder().method("POST").uri(path)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string())).unwrap();
    let response = crate::router(state.clone(), "/fixture-no-web").oneshot(req).await.unwrap();
    let status = response.status().as_u16();
    assert_eq!(response.headers()[axum::http::header::RETRY_AFTER], "1");
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let error: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(error["error"]["code"], "framerize_pool_busy");
    assert_eq!(error["error"]["retry_after_seconds"], 1);
    status
}

fn invalidate(ports: &Ports, change: u8) {
    let mut records = ports.records.lock().unwrap();
    match change {
        0 => records[0].cooling_until_ms = Some(u64::MAX),
        1 => records[0].framer_user_id = FramerUserId("changed-user".into()),
        2 => records[0].current_team = Some(TeamId("changed-team".into())),
        3 => records[0].current_project = Some(ProjectId("changed-project".into())),
        4 => { records.remove(0); },
        _ => unreachable!(),
    }
}

async fn reject_pool_mutations(state: &AppState, ports: &Ports) {
    assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1))]);
    let other = lease_account(state, id(2)).await.unwrap();
    let before = records(ports);
    for path in ["/api/pool/park", "/api/accounts/remove"] {
        assert_eq!(admin(state, path, json!({"account_id":id(1)})).await, 503);
    }
    for body in [json!({"active_capacity":1}), json!({"eligible_accounts":[id(2)]})] {
        assert_eq!(admin(state, "/api/pool/config", body).await, 503);
    }
    assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1)), ("status", id(2))]);
    assert_eq!(ports.setting_writes.load(Ordering::SeqCst), 0);
    assert_eq!(records(ports), before);
    drop(other);
}

impl n::OwnedInvocation for Call {
    fn raw(mut self: Box<Self>, _: Value) -> n::InvocationRun<n::RawResult> {
        self.admission.start();
        let Self { admission, permit, cleanup_gate, cleaned, fail_result, launched } = *self;
        launched.fetch_add(1, Ordering::SeqCst);
        let task = tokio::spawn(async move {
            cleanup_gate.acquire().await.unwrap().forget();
            admission.complete()?;
            drop(permit);
            cleaned.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        n::InvocationRun {
            result: async move {
                if fail_result { return Err(n::DispatchError::new(n::FailureKind::Transport)); }
                Ok(n::RawResult { body: b"{\"ok\":true}".to_vec(), metadata: n::AttemptMetadata {
                    status: 200, usage_header: None, original_content_type: Some("application/json".into()),
                    step_index: 0, elapsed: Duration::ZERO,
                } })
            }.boxed(),
            cleanup: async move { task.await.unwrap_or(Err(n::FailureKind::Cleanup)) }.boxed().shared(),
            cancellation: Arc::new(NoCancel), completed_attempts: None,
        }
    }
    fn messages(self: Box<Self>, _: Value) -> n::InvocationRun<n::Completed<AssembledResponse>> {
        panic!("paid raw fixture only")
    }
    fn responses(self: Box<Self>, _: Value) -> n::InvocationRun<n::Completed<AssembledResponses>> {
        panic!("paid raw fixture only")
    }
}

impl n::InvocationProvider for Probe {
    fn prepare(&self, _: n::Preparation, _: OwnedSemaphorePermit)
        -> BoxFuture<'_, Result<Box<dyn n::OwnedInvocation>, n::DispatchError>> {
        async { Err(n::DispatchError::new(n::FailureKind::InvalidScope)) }.boxed()
    }
    fn prepare_in_pool(&self, request: n::Preparation, permit: OwnedSemaphorePermit,
        admission: n::PoolAdmission)
        -> BoxFuture<'_, Result<Box<dyn n::OwnedInvocation>, n::DispatchError>> {
        async move {
            assert_eq!(request.identity.account, admission.account());
            self.preparations.fetch_add(1, Ordering::SeqCst);
            self.prepare_gate.acquire().await.unwrap().forget();
            Ok(Box::new(Call { admission, permit,
                cleanup_gate: self.cleanup_gates[usize::from(request.identity.account != id(1))].clone(),
                cleaned: self.cleaned.clone(), launched: self.launched.clone(),
                fail_result: self.fail_result.load(Ordering::SeqCst),
            }) as Box<dyn n::OwnedInvocation>)
        }.boxed()
    }
    fn supports_pool_admission(&self) -> bool { true }
    fn contains_account(&self, _: AccountId) -> bool { self.member.load(Ordering::SeqCst) }
    fn policy(&self) -> n::Policy {
        n::Policy { timeout: self.timeout, max_body_bytes: 1024,
            missing_content_type: "application/json".into(), chunk_tokens: 32,
            max_chunks: 1, continuation: None, allow_paid: true, credit_floor: 0.0 }
    }
}

fn native_fixture(timeout: Duration) -> (AppState, Arc<Ports>, Arc<Probe>) {
    let (mut state, ports) = fixture(2, 2);
    state.gate = Arc::new(framer_core::gate::ConcurrencyGate::with_config(
        framer_core::gate::GateConfig { initial: 1, min: 1, max: 1,
            recover_ms: 100000, relax_ms: 100000, min_interval_ms: 0 }));
    state = crate::operation_ownership::with_owned_operations(state, 32).unwrap();
    let probe = Arc::new(Probe { timeout, preparations: AtomicUsize::new(0),
        prepare_gate: Semaphore::new(0), cleanup_gates: std::array::from_fn(|_| Arc::new(Semaphore::new(0))),
        launched: Arc::new(AtomicUsize::new(0)),
        cleaned: Arc::new(AtomicUsize::new(0)), fail_result: AtomicBool::new(false),
        member: AtomicBool::new(true) });
    state.native_dispatch = Some(probe.clone());
    (state, ports, probe)
}

async fn request(state: AppState, hint: Option<u128>) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method("POST")
        .uri("/paid/openai/v1/responses").header("content-type", "application/json")
        .header("x-framerize-session", "pool-fixture-session");
    if let Some(n) = hint { builder = builder.header("x-framerize-account", id(n).0.to_string()); }
    let body = axum::body::Body::from(json!({"model":"gpt-4.1", "input":"fixture",
        "stream":false, "max_output_tokens":32}).to_string());
    crate::router(state, "/fixture-no-web").oneshot(builder.body(body).unwrap()).await.unwrap()
}

async fn block_model(state: &AppState, hint: Option<u128>)
    -> (OwnedSemaphorePermit, tokio::task::JoinHandle<axum::response::Response>) {
    let held = state.gate.acquire("team-1:openai/gpt-4.1").await;
    let task = tokio::spawn(request(state.clone(), hint));
    until(|| state.serving_accounts.residents.snapshot().unwrap().residents
        .iter().any(|slot| slot.account == id(1) && slot.users > 0)).await;
    (held, task)
}

#[tokio::test]
async fn native_rechecks_wave_and_provider_membership_after_model_wait() {
    for wave in [true, false] {
        let (state, ports, probe) = native_fixture(Duration::from_secs(3));
        let (held, task) = block_model(&state, Some(1)).await;
        if wave {
            state.wave.note_refusal("native-fixture-a");
            state.wave.note_refusal("native-fixture-b");
        } else { probe.member.store(false, Ordering::SeqCst); }
        drop(held);
        consume(task, if wave { 429 } else { 503 }).await;
        assert_eq!(probe.preparations.load(Ordering::SeqCst), 0);
        assert_eq!(slot(&state, id(1)).users, 0);
        assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1))]);
    }
}

#[tokio::test]
async fn native_rechecks_health_and_identity_after_model_wait_without_repicking() {
    for hint in [None, Some(1)] { for change in 0..5 {
        let (state, ports, probe) = native_fixture(Duration::from_secs(3));
        // Make automatic selection deterministic without resetting the shared cursor.
        ports.records.lock().unwrap()[1].consecutive_failures = 1;
        let (held, task) = block_model(&state, hint).await;
        invalidate(&ports, change);
        drop(held);
        let response = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
        let expected = if matches!(change, 0 | 4) { 503 } else { 409 };
        assert_eq!(response.status().as_u16(), expected);
        to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(probe.preparations.load(Ordering::SeqCst), 0);
        assert_eq!(slot(&state, id(1)).users, 0);
        assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1))]);
    } }
}

#[tokio::test]
async fn native_model_preparation_and_cleanup_hold_the_pool_mutation_fence() {
    for phase in 0..3 {
        let (state, ports, probe) = native_fixture(Duration::from_secs(5));
        let held = if phase == 0 { Some(state.gate.acquire("team-1:openai/gpt-4.1").await) } else { None };
        let task = tokio::spawn(request(state.clone(), Some(1)));
        until(|| state.serving_accounts.residents.snapshot().unwrap().residents
            .iter().any(|slot| slot.account == id(1) && slot.users > 0)).await;
        if phase > 0 { until(|| probe.preparations.load(Ordering::SeqCst) == 1).await; }
        if phase == 2 { probe.prepare_gate.add_permits(1); }
        if phase == 2 { until(|| probe.launched.load(Ordering::SeqCst) == 1).await; }
        reject_pool_mutations(&state, &ports).await;
        drop(held);
        if phase < 2 { probe.prepare_gate.add_permits(1); }
        probe.cleanup_gates[0].add_permits(1);
        let response = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
        assert_eq!(response.status(), 200);
        to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(slot(&state, id(1)).users, 0);
        assert!(!slot(&state, id(1)).uncertain);
    }
}
