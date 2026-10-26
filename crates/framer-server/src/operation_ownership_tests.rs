use super::*;
use axum::{Router, routing::post, body::Bytes, http::Request as HttpRequest};
use crate::response_output::consume_body as to_bytes;
use tower::ServiceExt;
use std::time::Duration;
use tokio::sync::{oneshot, Semaphore};
#[path = "permit_lifetime_fixture.rs"]
mod fixture;
const WAIT: Duration = Duration::from_secs(3);
fn scope() -> Arc<ServingOwnership> {
    ServingOwnership::new_unmanaged(Arc::new(WaveGuard::new()), 32).unwrap()
}
fn request(path: &str) -> HttpRequest<Body> {
    HttpRequest::builder().method("POST").uri(path)
        .header("content-type", "application/json").body(Body::from("{}" )).unwrap()
}
fn fixture_request(f: &fixture::Fixture, stream: bool) -> HttpRequest<Body> {
    HttpRequest::builder().method("POST").uri("/openai/v1/responses")
        .header("content-type", "application/json")
        .header("x-framerize-account", f.accounts[0].id.0.to_string())
        .header("x-framerize-session", "fixture-conversation")
        .body(Body::from(serde_json::json!({"model":"gpt-6-astra",
            "input":"fixture","stream":stream}).to_string())).unwrap()
}

#[path = "operation_ownership_nonstream_tests.rs"]
mod nonstream;

#[path = "operation_ownership_stream_recovery_tests.rs"]
mod stream_recovery;
#[path = "operation_ownership_extraction_tests.rs"]
mod extraction;
#[path = "operation_ownership_observation_tests.rs"]
mod observation;
#[path = "explicit_account_retry_tests.rs"]
mod explicit_account_retry;

#[tokio::test]
async fn every_mutating_and_inference_route_rejects_before_ports_after_seal() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    let before = f.state.serving_accounts.config();
    let _ticket = ownership.seal().unwrap();
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    for path in ["/api/run", "/api/accounts/begin", "/api/accounts/complete",
        "/api/accounts/import", "/api/accounts/remove", "/api/accounts/dedupe",
        "/api/pool/config", "/api/pool/activate", "/api/pool/park",
        "/anthropic/v1/messages", "/openai/v1/responses", "/openai/v1/chat/completions",
        "/paid/anthropic/v1/messages", "/paid/openai/v1/responses",
        "/paid/openai/v1/chat/completions"] {
        let response = app.clone().oneshot(request(path)).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
    }
    assert!(f.loads.try_recv().is_err());
    f.no_call();
    assert_eq!(f.state.serving_accounts.config(), before);
    assert_eq!(f.state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
    let health = app.oneshot(HttpRequest::builder().uri("/api/health").body(Body::empty()).unwrap())
        .await.unwrap();
    assert_eq!(health.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(health.headers()["x-framerize-pid"], std::process::id().to_string());
}

#[tokio::test]
async fn health_reports_unconfirmed_admission_without_opening_it() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let scope = f.state.operation_ownership.clone().unwrap();
    drop(scope.owner(OperationKind::Inference).unwrap());
    let before = scope.gate.stats().unwrap();
    let app = crate::router(f.state, "/nonexistent-fixture-web");
    let response = app.oneshot(HttpRequest::builder().uri("/api/health")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["x-framerize-pid"], std::process::id().to_string());
    assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
        b"operation admission closed");
    let after = scope.gate.stats().unwrap();
    assert_eq!(before.active, after.active);
    assert!(after.unconfirmed);
}

#[tokio::test]
async fn real_response_body_holds_admission_after_handler_and_producer_finish() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let task = tokio::spawn(app.oneshot(fixture_request(&f, false)));
    let call = f.next().await;
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(ownership.gate.stats().unwrap().active, 2); // host + body
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("fixture"));
    assert_eq!(ownership.gate.stats().unwrap().active, 1); // unresolved host remains
}

#[tokio::test]
async fn dropped_http_waiter_does_not_abort_admitted_auth_or_session_mutation() {
    let ownership = scope();
    let entered = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let completed = Arc::new(Semaphore::new(0));
    let (e, r, c) = (entered.clone(), release.clone(), completed.clone());
    let app = Router::new().route("/api/accounts/complete", post(move || {
        let (e, r, c) = (e.clone(), r.clone(), c.clone());
        async move {
            e.add_permits(1); r.acquire().await.unwrap().forget();
            c.add_permits(1); "mutation completed"
        }
    })).layer(axum::middleware::from_fn_with_state(Some(ownership.clone()), middleware));
    let request_task = tokio::spawn(app.oneshot(request("/api/accounts/complete")));
    tokio::time::timeout(WAIT, entered.acquire()).await.unwrap().unwrap().forget();
    request_task.abort(); let _ = request_task.await;
    let mut ticket = ownership.seal().unwrap();
    assert_eq!(ownership.gate.stats().unwrap().active, 2);
    release.add_permits(1);
    tokio::time::timeout(WAIT, completed.acquire()).await.unwrap().unwrap().forget();
    assert!(tokio::time::timeout(Duration::from_millis(1), ticket.wait()).await.is_err()
        || ownership.gate.stats().unwrap().unconfirmed);
}

#[tokio::test]
async fn unresolved_host_prevents_held_after_all_local_work_finishes() {
    let ownership = scope();
    run(Some(&ownership), OperationKind::Background, async {}).await.unwrap();
    let mut ticket = ownership.seal().unwrap();
    assert!(ownership.unmanaged_host_close_required());
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(tokio::time::timeout(Duration::from_millis(1), ticket.wait()).await.is_err());
    ticket.cancel_before_control().unwrap();
}

#[tokio::test]
async fn admitted_handler_registers_detached_cleanup_after_seal_and_retains_it() {
    let ownership = scope();
    let entered = Arc::new(Semaphore::new(0));
    let proceed = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let (handles, mut children) = tokio::sync::mpsc::unbounded_channel();
    let (o, e, p, r) = (ownership.clone(), entered.clone(), proceed.clone(), release.clone());
    let app = Router::new().route("/api/accounts/import", post(move || {
        let (o, e, p, r, handles) = (o.clone(), e.clone(), p.clone(), r.clone(), handles.clone());
        async move {
            e.add_permits(1); p.acquire().await.unwrap().forget();
            let child = spawn(Some(&o), OperationKind::Background, async move {
                r.acquire().await.unwrap().forget();
            }).unwrap();
            handles.send(child).unwrap(); "import complete"
        }
    })).layer(axum::middleware::from_fn_with_state(Some(ownership.clone()), middleware));
    let task = tokio::spawn(app.oneshot(request("/api/accounts/import")));
    tokio::time::timeout(WAIT, entered.acquire()).await.unwrap().unwrap().forget();
    let mut ticket = ownership.seal().unwrap();
    proceed.add_permits(1);
    let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    to_bytes(response.into_body(), 1024).await.unwrap();
    let child = children.recv().await.unwrap();
    assert_eq!(ownership.gate.stats().unwrap().active, 2);
    release.add_permits(1); child.await.unwrap();
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(tokio::time::timeout(Duration::from_millis(1), ticket.wait()).await.is_err());
}

#[tokio::test]
async fn real_stream_disconnect_keeps_inflight_work_but_stops_new_attempts() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let response = tokio::time::timeout(WAIT, app.clone().oneshot(fixture_request(&f, true)))
        .await.unwrap().unwrap();
    let call = f.next().await;
    assert_eq!(ownership.gate.stats().unwrap().active, 3);
    drop(response);
    let pool = app.clone().oneshot(HttpRequest::builder().uri("/api/pool")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(pool.status(), StatusCode::OK);
    to_bytes(pool.into_body(), 65536).await.unwrap();
    assert!(!call.reply.is_closed());
    let mut ticket = ownership.seal().unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(5),
        ticket.wait_application()).await.is_err());
    // This normally triggers account failover. Cancellation must suppress it.
    call.reply.send(Err("risk-refused".into())).ok().unwrap();
    tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
    let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
    ownership.check_application_drained(&proof).unwrap();
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
    f.no_call();
    drop(proof);
    ticket.cancel_before_control().unwrap();
}

struct Frames(std::collections::VecDeque<Frame<Bytes>>);

#[tokio::test]
async fn body_error_without_a_completion_observer_stays_unconfirmed() {
    let ownership = scope();
    let owner = ownership.owner(OperationKind::Inference).unwrap();
    let source = futures::stream::iter([Err::<Bytes, _>(std::io::Error::other("fixture"))]);
    let mut body = OwnedBody { body: Body::from_stream(source), owner: Some(owner), completion: None };
    let result = futures::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await;
    assert!(result.unwrap().is_err());
    assert!(matches!(ownership.seal(), Err(GateError::Unconfirmed)));
}

#[tokio::test]
async fn disconnected_browser_chain_retains_uncertainty_when_current_call_loses_cleanup() {
    for error in ["host-lifecycle-unconfirmed: fixture lost", "sidecar timeout after 900s (cmd ai_post)",
        "sidecar account command timed out (cmd ai_post)"] {
        let mut f = fixture::Fixture::new();
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let response = app.oneshot(fixture_request(&f, true)).await.unwrap();
        let call = f.next().await;
        drop(response);
        let mut ticket = ownership.seal().unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(5),
            ticket.wait_application()).await.is_err());
        call.reply.send(Err(error.into())).ok().unwrap();
        tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
        assert!(matches!(tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap(),
            Err(GateError::Unconfirmed)));
        assert!(ownership.gate.stats().unwrap().unconfirmed);
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert!(snapshot.residents.iter().any(|slot|
            slot.account == call.account && slot.uncertain));
        f.no_call();
    }
}

impl HttpBody for Frames {
    type Data = Bytes;
    type Error = std::convert::Infallible;
    fn poll_frame(mut self: Pin<&mut Self>, _: &mut Context<'_>)
        -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        Poll::Ready(self.0.pop_front().map(Ok))
    }
    fn is_end_stream(&self) -> bool { self.0.is_empty() }
}

#[tokio::test]
async fn body_wrapper_preserves_data_and_trailers_without_early_owner_release() {
    let ownership = scope();
    let owner = ownership.owner(OperationKind::Inference).unwrap();
    let mut trailers = axum::http::HeaderMap::new();
    trailers.insert("x-fixture-trailer", "fixture".parse().unwrap());
    let frames = Frames([Frame::data(Bytes::from_static(b"fixture")), Frame::trailers(trailers)]
        .into_iter().collect());
    let mut body = OwnedBody { body: Body::new(frames), owner: Some(owner),
        completion: None,
    };
    let first = futures::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await.unwrap().unwrap();
    assert_eq!(first.into_data().unwrap(), Bytes::from_static(b"fixture"));
    assert_eq!(ownership.gate.stats().unwrap().active, 2);
    let tail = futures::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await.unwrap().unwrap();
    assert_eq!(tail.into_trailers().unwrap()["x-fixture-trailer"], "fixture");
    drop(body);
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
}

#[tokio::test]
async fn startup_work_is_registered_before_spawn_and_new_startup_is_rejected_after_seal() {
    let ownership = scope();
    let (send, receive) = oneshot::channel::<()>();
    let prewarm = spawn(Some(&ownership), OperationKind::Background, async move {
        receive.await.unwrap();
    }).unwrap();
    let ticket = ownership.seal().unwrap();
    assert_eq!(ownership.gate.stats().unwrap().active, 2);
    assert!(matches!(spawn(Some(&ownership), OperationKind::Background, async {}), Err(GateError::Sealed)));
    send.send(()).unwrap(); prewarm.await.unwrap();
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    ticket.cancel_before_control().unwrap();
}

#[tokio::test]
async fn messages_cancel_cooperatively_but_unknown_paid_abort_stays_unconfirmed() {
    for messages in [true, false] {
        let mut f = fixture::Fixture::new();
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let path = if messages { "/anthropic/v1/messages" } else { "/paid/openai/v1/responses" };
        let payload = if messages {
            serde_json::json!({"model":"claude-opus-5-5","messages":[{"role":"user","content":"fixture"}],"stream":true})
        } else { serde_json::json!({"model":"gpt-6-astra","input":"fixture","stream":true}) };
        let req = HttpRequest::builder().method("POST").uri(path)
            .header("content-type", "application/json")
            .header("x-framerize-account", f.accounts[0].id.0.to_string())
            .body(Body::from(payload.to_string())).unwrap();
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let response = tokio::time::timeout(WAIT, app.oneshot(req)).await.unwrap().unwrap();
        let call = f.next().await;
        assert_eq!(call.kind, if messages { "messages" } else { "proxy" });
        assert_eq!(ownership.gate.stats().unwrap().active, 3);
        drop(response);
        if messages {
            assert!(!call.reply.is_closed());
            let mut ticket = ownership.seal().unwrap();
            assert!(tokio::time::timeout(Duration::from_millis(5),
                ticket.wait_application()).await.is_err());
            call.reply.send(Err("risk-refused".into())).ok().unwrap();
            tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
            let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
            ownership.check_application_drained(&proof).unwrap();
            assert!(!ownership.gate.stats().unwrap().unconfirmed);
            f.no_call();
            drop(proof);
            ticket.cancel_before_control().unwrap();
            continue;
        }
        tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
        tokio::time::timeout(WAIT, async {
            while ownership.gate.stats().unwrap().active > 1 { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert!(ownership.gate.stats().unwrap().unconfirmed);
    }
}

#[tokio::test]
async fn actual_access_log_observes_ownership_rejection_and_handler_panic() {
    for reject in [true, false] {
        let mut f = fixture::Fixture::new();
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let _ticket = if reject { Some(ownership.seal().unwrap()) } else { None };
        let capture = AccessLogCapture(Arc::new(Mutex::new(Vec::new())));
        // The real /run fixture denies inference and panics after admission.
        let mut req = HttpRequest::builder().method("POST").uri("/api/run")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"account_id":f.accounts[0].id,
                "prompt":"fixture","model":"fixture"}).to_string())).unwrap();
        req.extensions_mut().insert(capture.clone());
        let response = tokio::time::timeout(WAIT,
            crate::router(f.state.clone(), "/nonexistent-fixture-web").oneshot(req))
            .await.unwrap().unwrap();
        let expected = if reject { 503 } else { 500 };
        assert_eq!(response.status().as_u16(), expected);
        assert_eq!(*capture.0.lock().unwrap(), vec![expected]);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(body.as_ref(), if reject { b"operation admission closed".as_slice() }
            else { b"operation completion unconfirmed".as_slice() });
        if reject { assert!(f.loads.try_recv().is_err()); }
        else { assert_eq!(f.loads.try_recv().unwrap(), f.accounts[0].framer_user_id); }
    }
}

#[tokio::test]
async fn real_body_blocks_application_observation_without_releasing_host() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let task = tokio::spawn(app.oneshot(fixture_request(&f, false)));
    let call = f.next().await;
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    let mut ticket = ownership.seal().unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(2),
        ticket.wait_application()).await.is_err());
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("fixture"));
    let proof = ticket.wait_application().await.unwrap();
    ownership.check_application_drained(&proof).unwrap();
    let other = scope();
    assert_eq!(other.check_application_drained(&proof), Err(GateError::WrongGate));
    drop(proof);
    assert!(tokio::time::timeout(Duration::from_millis(2), ticket.wait()).await.is_err());
    assert!(ownership.unmanaged_host_close_required());
}

#[tokio::test]
async fn application_wait_timeout_keeps_detached_serving_job_owned() {
    let ownership = scope();
    let (release, released) = oneshot::channel();
    let (done, completed) = oneshot::channel();
    let task = spawn(Some(&ownership), OperationKind::StateWrite, async move {
        released.await.unwrap();
        done.send(()).unwrap();
    }).unwrap();
    drop(task); // Dropped waiter must not cancel admitted work.
    let mut ticket = ownership.seal().unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(2),
        ticket.wait_application()).await.is_err());
    release.send(()).unwrap();
    tokio::time::timeout(WAIT, completed).await.unwrap().unwrap();
    let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
    ownership.check_application_drained(&proof).unwrap();
    drop(proof);
    assert!(tokio::time::timeout(Duration::from_millis(2), ticket.wait()).await.is_err());
}
