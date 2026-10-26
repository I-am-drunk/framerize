//! Exercise real finalization after a synthetic completed request.
use super::*;

struct Ready {
    state: ServiceState, ports: Arc<Ports>, spec: Specification, sessions: Sessions,
    completion: Option<native_control::ControlOwner>, pending: Option<PendingSession>,
    scope: AccountScope,
}

fn completed() -> Outcome {
    Outcome { response: Some(CompletedJson::Responses(framer_core::convert::AssembledResponses {
        id: "fixture-result".into(), created_at: 123, status: "completed".into(),
        incomplete_details: None, service_tier: serde_json::Value::Null,
        output: vec![serde_json::json!({"type":"message","content":[]})],
        usage: serde_json::json!({"input_tokens":1,"output_tokens":1}),
        input_tokens: 1, output_tokens: 1, chunks: 1, metered_chunks: 0,
    })), evidence: None, failure: None, cleanup_confirmed: true }
}

async fn ready() -> Ready {
    let (state, ports, spec, sessions) = fixture();
    let AdmittedControl { capacity, pool, mut completion, .. } =
        native_control::admit(&state, spec.identity.clone(), "gpt-4.1", spec.deadline)
            .await.unwrap();
    completion.start();
    drop(capacity);
    pool.complete().unwrap();
    let lease = sessions.bridge().lease_account_observed(spec.identity.account,
        spec.identity.user.clone(), NativeHttpLimits::default(), NativeEgress::Direct)
        .await.unwrap();
    let scope = lease.scope().clone();
    let device = DeviceLease::open(&device::DeviceBuilder(ports.clone()),
        spec.identity.account).await.unwrap();
    Ready { state, ports, spec, sessions, completion: Some(completion), scope,
        pending: Some(PendingSession {
            outcome: completed(), retained: Some(lease), device: Some(device) }) }
}

async fn finalize(r: &mut Ready, stop: &Stop) -> Outcome {
    tokio::time::timeout(Duration::from_secs(2), finish_owned(&r.state, &r.spec.identity,
        &None, r.completion.take().unwrap(), stop, r.spec.deadline,
        r.pending.take().unwrap())).await.unwrap()
}

#[tokio::test]
async fn completed_device_and_publication_allow_transport_reuse() {
    let mut r = ready().await;
    let stop = Stop(watch::channel(false).0);
    let outcome = finalize(&mut r, &stop).await;
    assert!(outcome.cleanup_confirmed && outcome.failure.is_none());
    assert!(outcome.response.is_some());
    assert!(r.scope.closed().now_or_never().is_none());
    assert_eq!(r.ports.device_users.load(Ordering::SeqCst), 0);
    r.ports.read.add_permits(1);
    let next = r.sessions.bridge().lease_account_observed(r.spec.identity.account,
        r.spec.identity.user, NativeHttpLimits::default(), NativeEgress::Direct).await.unwrap();
    assert!(r.scope.same_generation(next.scope()));
    assert!(next.finish(true).await);
    r.sessions.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_or_panicking_device_close_retires_the_completed_transport() {
    for mode in [3, 4] {
        let mut r = ready().await;
        r.ports.device_mode.store(mode, Ordering::SeqCst);
        let outcome = finalize(&mut r, &Stop(watch::channel(false).0)).await;
        assert_eq!(outcome.failure, Some(FailureKind::Cleanup));
        assert!(!outcome.cleanup_confirmed && outcome.response.is_none());
        assert_eq!(r.scope.closed().now_or_never(), Some(Ok(())));
        assert_eq!(r.ports.device_users.load(Ordering::SeqCst), 1);
        r.sessions.bridge().shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn publication_failure_retires_the_completed_transport() {
    let mut r = ready().await;
    r.ports.account.lock().unwrap().proxy = Some("http://127.0.0.1:9".into());
    let outcome = finalize(&mut r, &Stop(watch::channel(false).0)).await;
    assert_eq!(outcome.failure, Some(FailureKind::InvalidScope));
    assert!(outcome.cleanup_confirmed && outcome.response.is_none());
    assert_eq!(r.scope.closed().now_or_never(), Some(Ok(())));
    r.sessions.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancellation_during_device_close_retires_the_completed_transport() {
    let mut r = ready().await;
    let ports = r.ports.clone();
    let scope = r.scope.clone();
    ports.device_finish.try_acquire().unwrap().forget();
    let stop = Stop(watch::channel(false).0);
    let mut finishing = Box::pin(finalize(&mut r, &stop));
    assert!(futures::poll!(finishing.as_mut()).is_pending());
    wait(&ports.device_closing).await;
    assert!(scope.closed().now_or_never().is_none());
    stop.cancel();
    ports.device_finish.add_permits(1);
    let outcome = finishing.await;
    assert_eq!(outcome.failure, Some(FailureKind::Cancelled));
    assert!(outcome.cleanup_confirmed && outcome.response.is_none());
    assert_eq!(scope.closed().now_or_never(), Some(Ok(())));
    r.sessions.shutdown().await.unwrap();
}

#[tokio::test]
async fn abandoning_finalization_cannot_leave_a_reusable_generation() {
    let mut r = ready().await;
    let ports = r.ports.clone();
    let scope = r.scope.clone();
    ports.device_finish.try_acquire().unwrap().forget();
    let stop = Stop(watch::channel(false).0);
    let mut finishing = Box::pin(finalize(&mut r, &stop));
    assert!(futures::poll!(finishing.as_mut()).is_pending());
    wait(&ports.device_closing).await;
    drop(finishing);
    tokio::time::timeout(Duration::from_secs(2), scope.closed()).await.unwrap().unwrap();
    assert_eq!(ports.device_users.load(Ordering::SeqCst), 1);
    assert!(native_control::admit(&r.state, r.spec.identity.clone(), "gpt-4.1",
        Instant::now() + Duration::from_secs(1)).await.is_err());
    r.sessions.bridge().shutdown().await.unwrap();
}
