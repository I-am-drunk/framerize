include!("composed_fixture.rs");

#[test]
fn composed_refusal_failed_native_cleanup_keeps_capacity_quarantined() {
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", expected_responses(), 0,
        refusal(b"cleanup failure fixture".to_vec()))]);
    let (capacity, supervisor) = run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = composed_hooked(&f, deadline(), options(ClientDialect::OpenAiResponses),
            TestHooks { lose_native_closed: true, ..TestHooks::default() });
        let error = started.response.result.await.err().unwrap();
        assert_eq!(error.kind, FailureKind::Cleanup); assert!(error.upstream.is_none());
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::Cleanup));
        assert_eq!(started.response.cleanup.await, Err(FailureKind::Cleanup));
        let capacity = f.capacity.clone(); let supervisor = f.client.supervisor_for_test();
        drop(f); assert_eq!(capacity.available_permits(), 0); assert!(!supervisor.is_finished());
        (capacity, supervisor)
    });
    assert_eq!(capacity.available_permits(), 1); assert!(supervisor.is_finished());
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_refusal_cancellation_cannot_release_a_held_error_body() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
        refusal(b"held after cancellation".to_vec()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), deadline()).await;
        let error = started.response.result.await.err().unwrap();
        assert_eq!(error.kind, FailureKind::Upstream);
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::Upstream));
        started.response.cancellation.cancel();
        assert!(tokio::time::timeout(PAUSE, started.response.cleanup.clone()).await.is_err());
        assert_eq!(f.capacity.available_permits(), 0);
        assert_eq!(error.upstream.as_ref().unwrap().body.as_ref(), b"held after cancellation");
        drop(error); cleanup(started.response.cleanup).await; f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_refusal_oversize_releases_json_lease_without_returning_a_body() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
        refusal(vec![b'x'; 129]))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let mut requested = options(ClientDialect::OpenAiResponses);
        requested.limits.json_bytes = 128;
        let started = compose(&f, Route::Responses, responses_payload(), requested, deadline()).await;
        let error = started.response.result.await.err().unwrap();
        assert_eq!(error.kind, FailureKind::BodyTooLarge); assert!(error.upstream.is_none());
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::BodyTooLarge));
        cleanup(started.response.cleanup).await; f.finish().await;
    });
    let observed = server.finish();
    assert_eq!(observed.fault, None); assert_eq!(observed.requests, 1);
    assert_eq!(observed.headers, 1); assert!(observed.replies <= 1);
}

#[test]
fn composed_refusal_holding_one_account_does_not_block_another() {
    let server = Server::new(vec![
        expected("/ai/v3/responses/", expected_responses(), 0, refusal(b"held refusal".to_vec())),
        expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(response_body()))]);
    run(async {
        let held = Fixture::new(&server, Supplier::default(), policy());
        let other = Fixture::new(&server, Supplier::default(), policy());
        assert_ne!(held.scope.account(), other.scope.account());
        let first = compose(&held, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), deadline()).await;
        let error = first.response.result.await.err().unwrap();
        assert_eq!(error.kind, FailureKind::Upstream);
        assert_eq!(summary(first.finished).await.result, Err(FailureKind::Upstream));
        assert!(held.capacity.clone().try_acquire_owned().is_err());
        let (_, done) = composed_finished(compose(&other, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), deadline()).await).await;
        assert_eq!(done.result, Ok(())); assert_eq!(other.capacity.available_permits(), 1);
        assert_eq!(held.capacity.available_permits(), 0);
        drop(error); cleanup(first.response.cleanup).await;
        let next = held.capacity.clone().try_acquire_owned().unwrap(); drop(next);
        held.finish().await; other.finish().await;
    });
    checked(&server.finish(), 2, 2);
}

#[test]
fn composed_refusal_dropped_pending_result_waits_for_actual_native_cleanup() {
    let mut reply = refusal(b"body remains gated".to_vec()); reply.gate_body = true;
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", expected_responses(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let (release, tail) = oneshot::channel();
        let started = composed_hooked(&f, deadline(), options(ClientDialect::OpenAiResponses),
            TestHooks { tail_gate: Some(tail), ..TestHooks::default() });
        recorded(&started).await;
        drop(started.response.result);
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::Cancelled));
        assert!(tokio::time::timeout(PAUSE, started.response.cleanup.clone()).await.is_err());
        assert_eq!(f.capacity.available_permits(), 0);
        release.send(()).unwrap();
        cleanup(started.response.cleanup).await;
        wait_count(&server.shared.peer_eof, 1).await; f.finish().await;
    });
    checked(&server.finish(), 1, 0);
}

#[test]
fn composed_refusal_requires_native_cleanup_then_last_body_owner() {
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", expected_responses(), 0,
        refusal(b"complete but native cleanup gated".to_vec()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let (release, tail) = oneshot::channel();
        let started = composed_hooked(&f, deadline(), options(ClientDialect::OpenAiResponses),
            TestHooks { tail_gate: Some(tail), ..TestHooks::default() });
        recorded(&started).await;
        wait_count(&server.shared.replies, 1).await;
        let mut result = started.response.result;
        assert!(tokio::time::timeout(PAUSE, &mut result).await.is_err());
        assert!(started.response.cleanup.clone().now_or_never().is_none());
        assert_eq!(f.capacity.available_permits(), 0);
        release.send(()).unwrap();
        let error = tokio::time::timeout(STEP, result).await.unwrap().err().unwrap();
        assert_eq!(error.kind, FailureKind::Upstream);
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::Upstream));
        assert!(tokio::time::timeout(PAUSE, started.response.cleanup.clone()).await.is_err());
        assert_eq!(f.capacity.available_permits(), 0);
        drop(error); cleanup(started.response.cleanup).await; f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_refusal_unpolled_error_handoff_retains_then_releases_capacity() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
        refusal(b"unpolled original refusal".to_vec()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), deadline()).await;
        // The worker has successfully sent the error, but result is unpolled.
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::Upstream));
        assert_eq!(f.capacity.available_permits(), 0);
        assert!(tokio::time::timeout(PAUSE, started.response.cleanup.clone()).await.is_err());
        drop(started.response.result);
        cleanup(started.response.cleanup).await; f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_refusal_empty_owned_body_keeps_lease_until_last_clone() {
    let server = Server::new(vec![expected("/ai/v3/messages/", expected_messages(), 0,
        refusal(Vec::new()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = compose(&f, Route::Messages, messages_payload(),
            options(ClientDialect::AnthropicMessages), deadline()).await;
        let error = started.response.result.await.err().unwrap();
        assert_eq!(error.kind, FailureKind::Upstream);
        let upstream = error.upstream.unwrap();
        assert!(upstream.body.is_empty());
        let cloned = upstream.body.clone(); let sliced = upstream.body.slice(..0);
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::Upstream));
        drop(upstream);
        assert!(tokio::time::timeout(PAUSE, started.response.cleanup.clone()).await.is_err());
        assert_eq!(f.capacity.available_permits(), 0);
        drop(cloned); cleanup(started.response.cleanup).await;
        assert!(sliced.is_empty()); drop(sliced); f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

fn refusal(body: Vec<u8>) -> Reply {
    let mut reply = Reply::json(Value::Null);
    reply.status = 403; reply.body = body;
    reply.content_type = Some("application/x-refusal-fixture");
    reply
}

async fn recorded(run: &ComposedStreamRun) {
    tokio::time::timeout(STEP, async {
        while run.response.completed_attempts.as_ref().unwrap()().is_empty() {
            tokio::time::sleep(TICK).await;
        }
    }).await.unwrap();
}

#[test]
fn composed_refusal_last_clone_and_slice_hold_original_capacity() {
    let wire = vec![0, 255, b'x', b'y', b'z'];
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
        refusal(wire.clone()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), deadline()).await;
        let error = started.response.result.await.err().unwrap();
        assert_eq!(error.kind, FailureKind::Upstream);
        let upstream = error.upstream.unwrap();
        assert_eq!(upstream.body.as_ref(), wire.as_slice());
        assert_eq!(upstream.metadata.status, 403);
        let copied = upstream.body.clone();
        let sliced = upstream.body.slice(1..3);
        let empty = upstream.body.slice(2..2);
        assert!(empty.is_empty());
        let done = summary(started.finished).await;
        assert_eq!(done.result, Err(FailureKind::Upstream));
        assert_eq!(done.emitted_bytes, 0); assert_eq!(done.attempts.len(), 1);
        assert!(f.capacity.clone().try_acquire_owned().is_err());
        assert!(tokio::time::timeout(PAUSE, started.response.cleanup.clone()).await.is_err());
        drop(upstream);
        assert_eq!(f.capacity.available_permits(), 0);
        drop(copied);
        assert!(tokio::time::timeout(PAUSE, started.response.cleanup.clone()).await.is_err());
        assert_eq!(sliced.as_ref(), &wire[1..3]);
        drop(sliced);
        // Empty slices do not retain backing storage in the pinned bytes crate.
        cleanup(started.response.cleanup).await;
        assert_eq!(f.capacity.available_permits(), 1);
        assert!(empty.is_empty()); drop(empty); f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}
