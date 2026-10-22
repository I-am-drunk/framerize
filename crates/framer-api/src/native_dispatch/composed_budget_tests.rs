include!("composed_fixture.rs");

#[test]
fn composed_dropped_head_stops_before_authorization() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = compose(&f, Route::Responses, responses_payload(), options(ClientDialect::OpenAiResponses), deadline()).await;
        drop(started.response.result);
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::Cancelled));
        cleanup(started.response.cleanup).await;
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0); f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn composed_cancel_discards_queue_but_waits_for_borrowed_bytes() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(response_body()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = compose(&f, Route::Responses, responses_payload(), options(ClientDialect::OpenAiResponses), deadline()).await;
        let mut head = started.response.result.await.unwrap();
        let held = head.body.next().await.unwrap().unwrap();
        started.response.cancellation.cancel();
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::Cancelled));
        assert_eq!(head.body.next().await, Some(Err(FailureKind::Cancelled)));
        assert!(started.response.cleanup.clone().now_or_never().is_none());
        assert_eq!(f.capacity.available_permits(), 0);
        drop(held); cleanup(started.response.cleanup).await; f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_chat_suffix_budget_failure_precedes_any_success_marker() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(minimal_body()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let mut requested = options(ClientDialect::OpenAiChat); requested.limits.frame_bytes = 300;
        let started = compose(&f, Route::Responses, responses_payload(), requested, deadline()).await;
        let head = started.response.result.await.unwrap();
        let (bytes, error) = composed_outcome(head.body).await;
        assert_eq!(error, Some(FailureKind::BodyTooLarge));
        assert!(events(&bytes).iter().all(|event| event["choices"][0]["finish_reason"].is_null()));
        assert!(!bytes.windows(6).any(|window| window == b"[DONE]"));
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::BodyTooLarge));
        cleanup(started.response.cleanup).await; f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_slow_account_does_not_spend_other_accounts_output_credits() {
    let request = expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(response_body()));
    let server = Server::new(vec![request.clone(), request]);
    run(async {
        let slow = Fixture::new(&server, Supplier::default(), policy());
        let fast = Fixture::new(&server, Supplier::default(), policy());
        let mut requested = options(ClientDialect::OpenAiResponses);
        requested.limits.retained_bytes = requested.limits.frame_bytes;
        let started = compose(&slow, Route::Responses, responses_payload(), requested, deadline()).await;
        let mut head = started.response.result.await.unwrap();
        let held = head.body.next().await.unwrap().unwrap();
        assert!(tokio::time::timeout(PAUSE, head.body.next()).await.is_err());
        let (_, done) = composed_finished(compose(&fast, Route::Responses, responses_payload(), requested, deadline()).await).await;
        assert_eq!(done.result, Ok(())); assert_eq!(fast.capacity.available_permits(), 1);
        assert_eq!(slow.capacity.available_permits(), 0); assert_ne!(slow.scope.account(), fast.scope.account());
        drop(held); let _ = composed_bytes(head.body).await;
        assert_eq!(summary(started.finished).await.result, Ok(()));
        cleanup(started.response.cleanup).await; slow.finish().await; fast.finish().await;
    });
    checked(&server.finish(), 2, 2);
}

#[test]
fn composed_scope_close_during_continuation_keeps_original_generation() {
    let (first, next) = responses_continuation();
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(first))]);
    run(async {
        let mut configured = policy(); configured.continuation = Some(CALLER_CONTINUE.into());
        let f = Fixture::new(&server, Supplier { never: true, gate_index: 1, ..Supplier::default() }, configured);
        let until = deadline();
        let started = compose(&f, Route::Responses, responses_payload(), options(ClientDialect::OpenAiResponses), until).await;
        let head = started.response.result.await.unwrap();
        let body = tokio::spawn(composed_outcome(head.body));
        wait_count(&f.supplier.calls, 2).await; close(&f.client, &f.scope).await;
        let (bytes, error) = body.await.unwrap(); no_success(&events(&bytes));
        assert_eq!(error, Some(FailureKind::ScopeClosed));
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::ScopeClosed));
        cleanup(started.response.cleanup).await;
        seen(&f.supplier, &f.scope, until, "OpenRouterResponses", &[expected_responses(), next]);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_failed_native_cleanup_keeps_original_capacity_quarantined() {
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", expected_responses(), 0, Reply::json(response_body()))]);
    let (capacity, supervisor) = run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = composed_hooked(&f, deadline(), options(ClientDialect::OpenAiResponses),
            TestHooks { lose_native_closed: true, ..TestHooks::default() });
        let head = started.response.result.await.unwrap();
        let (bytes, body_error) = composed_outcome(head.body).await;
        no_success(&events(&bytes));
        if let Some(kind) = body_error { assert_eq!(kind, FailureKind::Cleanup); }
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
fn composed_headers_share_one_budget_across_attempts() {
    let (first, next) = responses_continuation();
    let server = Server::new(vec![
        expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(first)),
        expected("/ai/v3/responses/", next, 1, Reply::json(response_body()))]);
    run(async {
        let mut configured = policy(); configured.continuation = Some(CALLER_CONTINUE.into());
        let f = Fixture::new(&server, Supplier::default(), configured);
        let mut requested = options(ClientDialect::OpenAiResponses);
        // Admit the first header's 80-byte backing allocation, but not both
        // attempts' 49-byte retained headers. Failure must be on continuation.
        let retained = "remaining=7.5&fixture=application".len() + "application/json".len();
        requested.limits.header_bytes = 2 * retained - 1;
        let (bytes, done) = composed_finished(compose(&f, Route::Responses, responses_payload(), requested, deadline()).await).await;
        assert_eq!(done.result, Err(FailureKind::BodyTooLarge));
        no_success(&events(&bytes)); assert_eq!(done.attempts.len(), 1);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 2); f.finish().await;
    });
    let snapshot = server.finish(); assert_eq!(snapshot.fault, None);
    assert_eq!(snapshot.requests, 2); assert_eq!(snapshot.headers, 2);
    assert!((1..=2).contains(&snapshot.replies));
}

#[test]
fn composed_body_drop_reports_before_delayed_real_cleanup() {
    let mut reply = Reply::json(response_body()); reply.gate_body = true;
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", expected_responses(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let (release, tail) = oneshot::channel();
        let started = composed_hooked(&f, deadline(), options(ClientDialect::OpenAiResponses),
            TestHooks { tail_gate: Some(tail), ..TestHooks::default() });
        let head = started.response.result.await.unwrap(); drop(head);
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::Cancelled));
        assert!(started.response.cleanup.clone().now_or_never().is_none());
        assert_eq!(f.capacity.available_permits(), 0);
        release.send(()).unwrap(); cleanup(started.response.cleanup).await;
        wait_count(&server.shared.peer_eof, 1).await; f.finish().await;
    });
    checked(&server.finish(), 1, 0);
}

#[test]
fn composed_original_deadline_cancels_pending_authorization() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier { never: true, ..Supplier::default() }, policy());
        let until = Instant::now() + Duration::from_millis(500);
        let started = compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), until).await;
        assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::Deadline);
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::Deadline));
        cleanup(started.response.cleanup).await;
        assert_eq!(f.supplier.cancelled.load(Ordering::Acquire), 1);
        seen(&f.supplier, &f.scope, until, "OpenRouterResponses", &[expected_responses()]);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn composed_terminal_error_credit_survives_eof_and_clones() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(response_body()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let mut requested = options(ClientDialect::OpenAiResponses); requested.limits.frame_bytes = 128;
        let started = compose(&f, Route::Responses, responses_payload(), requested, deadline()).await;
        let mut head = started.response.result.await.unwrap();
        let bytes = head.body.next().await.unwrap().unwrap();
        assert_eq!(events(&bytes)[0]["type"], "error");
        let copy = bytes.clone(); let slice = bytes.slice(..1); drop(bytes);
        assert_eq!(head.body.next().await, None);
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::BodyTooLarge));
        assert!(started.response.cleanup.clone().now_or_never().is_none());
        drop(copy); assert!(started.response.cleanup.clone().now_or_never().is_none());
        drop(slice); cleanup(started.response.cleanup).await; f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_unpolled_body_expires_and_discards_queued_frames() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(response_body()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), Instant::now() + Duration::from_millis(500)).await;
        let mut head = started.response.result.await.unwrap();
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::Deadline));
        cleanup(started.response.cleanup).await;
        assert_eq!(head.body.next().await, Some(Err(FailureKind::Deadline)));
        assert_eq!(head.body.next().await, None); f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_byte_and_frame_credits_follow_last_clone_and_slice() {
    for count_limited in [false, true] {
        let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(response_body()))]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let mut requested = options(ClientDialect::OpenAiResponses);
            if count_limited { requested.limits.retained_frames = 1; }
            else { requested.limits.retained_bytes = requested.limits.frame_bytes; }
            let started = compose(&f, Route::Responses, responses_payload(), requested, deadline()).await;
            let mut head = started.response.result.await.unwrap();
            let first = head.body.next().await.unwrap().unwrap();
            let copied = first.clone(); let sliced = first.slice(..1); drop(first);
            assert!(tokio::time::timeout(PAUSE, head.body.next()).await.is_err());
            drop(copied);
            assert!(tokio::time::timeout(PAUSE, head.body.next()).await.is_err());
            drop(sliced); let _ = composed_bytes(head.body).await;
            assert_eq!(summary(started.finished).await.result, Ok(()));
            cleanup(started.response.cleanup).await; f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

#[test]
fn composed_success_cleanup_waits_for_bytes_retained_after_eof() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(response_body()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let mut requested = options(ClientDialect::OpenAiResponses);
        requested.limits.retained_frames = 64;
        requested.limits.retained_bytes = requested.limits.frame_bytes * 64;
        let started = compose(&f, Route::Responses, responses_payload(), requested, deadline()).await;
        let mut head = started.response.result.await.unwrap();
        let held = head.body.next().await.unwrap().unwrap();
        let _ = composed_bytes(head.body).await;
        assert_eq!(summary(started.finished).await.result, Ok(()));
        assert!(started.response.cleanup.clone().now_or_never().is_none());
        assert_eq!(f.capacity.available_permits(), 0);
        drop(held); cleanup(started.response.cleanup).await; f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_parsed_container_budget_is_separate_from_wire_length() {
    let payload = json!({"model":"gpt-4.1","input":"small","stream":false});
    let prepared = json!({"model":"openai/gpt-4.1","stream":false,"max_output_tokens":32,
        "input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"small"}]}]});
    let mut body = response_body(); body["many_nodes"] = Value::Array(vec![Value::Null; 400]);
    assert!(serde_json::to_vec(&body).unwrap().len() < 16 * 1024);
    let server = Server::new(vec![expected("/ai/v3/responses/", prepared, 0, Reply::json(body))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let mut requested = options(ClientDialect::OpenAiResponses); requested.limits.state_bytes = 512 * 1024;
        let (bytes, done) = composed_finished(compose(&f, Route::Responses, payload, requested, deadline()).await).await;
        assert_eq!(done.result, Err(FailureKind::BodyTooLarge));
        no_success(&events(&bytes)); assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_frame_and_wire_limits_fail_after_real_headers() {
    for wire_limit in [false, true] {
        let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(response_body()))]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let mut requested = options(ClientDialect::OpenAiResponses);
            if wire_limit { requested.limits.json_bytes = 128; }
            else { requested.limits.frame_bytes = 128; }
            let started = compose(&f, Route::Responses, responses_payload(), requested, deadline()).await;
            let head = started.response.result.await.unwrap(); assert_eq!(head.first_attempt.status, 200);
            let emitted = events(&composed_bytes(head.body).await); no_success(&emitted);
            assert_eq!(emitted.last().unwrap()["type"], "error");
            assert_eq!(summary(started.finished).await.result, Err(FailureKind::BodyTooLarge));
            cleanup(started.response.cleanup).await; f.finish().await;
        });
        let snapshot = server.finish();
        assert_eq!(snapshot.fault, None); assert_eq!(snapshot.requests, 1);
        assert_eq!(snapshot.headers, 1);
        if wire_limit { assert!(snapshot.replies <= 1); }
        else { assert_eq!(snapshot.replies, 1); }
    }
}

#[test]
fn composed_total_output_limit_keeps_a_terminal_error_reserve() {
    let mut body = minimal_body(); body["output"][0]["id"] = json!("msg_large");
    body["output"][0]["content"][0]["text"] = json!("x".repeat(3000));
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(body))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let mut requested = options(ClientDialect::OpenAiResponses);
        requested.limits.total_bytes = requested.limits.frame_bytes + requested.limits.terminal_bytes;
        let (bytes, done) = composed_finished(compose(&f, Route::Responses, responses_payload(), requested, deadline()).await).await;
        assert!(bytes.len() <= requested.limits.total_bytes);
        let emitted = events(&bytes); no_success(&emitted);
        assert!(emitted.iter().any(|event| event["type"] == "response.output_text.delta"));
        assert_eq!(emitted.last().unwrap()["type"], "error");
        assert_eq!(done.result, Err(FailureKind::BodyTooLarge)); f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_invalid_limits_and_dialect_fail_before_authorization() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        for index in 0..9 {
            let mut requested = options(ClientDialect::OpenAiResponses);
            match index {
                0 => requested.limits.frame_bytes = 0,
                1 => requested.limits.retained_bytes = 1,
                2 => requested.limits.retained_frames = 0,
                3 => requested.limits.state_bytes = 0,
                4 => requested.limits.json_bytes = 0,
                5 => requested.limits.source_chunk_bytes = 1024 * 1024 + 1,
                6 => requested.limits.header_bytes = 0,
                7 => requested.limits.terminal_bytes = 511,
                _ => requested.dialect = ClientDialect::AnthropicMessages,
            }
            let started = compose(&f, Route::Responses, responses_payload(), requested, deadline()).await;
            assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::InvalidRequest);
            assert_eq!(summary(started.finished).await.result, Err(FailureKind::InvalidRequest));
            cleanup(started.response.cleanup).await;
        }
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0); f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn composed_policy_copies_are_admitted_before_authorization() {
    let server = Server::new(vec![]);
    run(async {
        let mut configured = policy(); configured.max_chunks = 64;
        configured.continuation = Some("x".repeat(256 * 1024));
        let f = Fixture::new(&server, Supplier::default(), configured);
        let started = compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), deadline()).await;
        assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::BodyTooLarge);
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::BodyTooLarge));
        cleanup(started.response.cleanup).await;
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0); f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}
