include!("raw_stream_fixture.rs");

#[test]
fn raw_stream_slow_account_does_not_consume_other_accounts_credits() {
    let mut reply = Reply::json(json!(null)); reply.body = b"independent account bytes".to_vec();
    let bytes = reply.body.clone();
    let expected = expected("/ai/v3/openai/responses/", stream_payload(), 0, reply);
    let server = Server::new(vec![expected.clone(), expected]);
    run(async {
        let slow = Fixture::new(&server, Supplier::default(), policy());
        let fast = Fixture::new(&server, Supplier::default(), policy());
        assert_ne!(slow.scope.account(), fast.scope.account());
        let mut bound = limits(); bound.retained_bytes = bound.chunk_bytes;
        let first = start(&slow, deadline(), bound).await;
        let second = start(&fast, deadline(), bound).await;
        let mut slow_head = first.response.result.await.unwrap();
        let held = slow_head.body.next().await.unwrap().unwrap();
        let mut received = held.to_vec();
        assert!(tokio::time::timeout(PAUSE, slow_head.body.next()).await.is_err());
        let fast_head = second.response.result.await.unwrap();
        assert_eq!(drain(fast_head.body).await, bytes);
        assert_eq!(finished(second.finished).await.result, Ok(()));
        cleanup(second.response.cleanup).await;
        assert_eq!(fast.capacity.available_permits(), 1);
        assert_eq!(slow.capacity.available_permits(), 0);
        drop(held);
        received.extend(drain(slow_head.body).await);
        assert_eq!(received, bytes);
        assert_eq!(finished(first.finished).await.result, Ok(()));
        cleanup(first.response.cleanup).await;
        slow.finish().await; fast.finish().await;
    });
    checked(&server.finish(), 2, 2);
}

#[test]
fn raw_stream_deadline_reports_before_delayed_real_cleanup() {
    let mut reply = Reply::json(json!(null)); reply.gate_body = true;
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let (release, tail) = oneshot::channel();
        let started = hooked_stream(&f, Instant::now() + Duration::from_millis(500),
            TestHooks { tail_gate: Some(tail), ..TestHooks::default() });
        let mut head = started.response.result.await.unwrap();
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::Deadline));
        assert!(started.response.cleanup.clone().now_or_never().is_none());
        assert_eq!(f.capacity.available_permits(), 0);
        assert_eq!(head.body.next().await, Some(Err(FailureKind::Deadline)));
        release.send(()).unwrap();
        cleanup(started.response.cleanup).await;
        wait_count(&server.shared.peer_eof, 1).await;
        f.finish().await;
    });
    checked(&server.finish(), 1, 0);
}

#[test]
fn raw_stream_output_does_not_pin_large_foreign_backing() {
    for visible in [3, 6] {
        let mut reply = Reply::json(json!(null)); reply.gate_body = true;
        let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let dropped = Arc::new(AtomicUsize::new(0));
            let started = foreign_stream(&f, visible, dropped.clone());
            let mut head = started.response.result.await.unwrap();
            let item = head.body.next().await.unwrap();
            assert_eq!(dropped.load(Ordering::Acquire), 1);
            if visible == 3 {
                let bytes = item.unwrap(); assert_eq!(bytes.as_ref(), &[42; 3]);
                drop(head);
                assert_eq!(finished(started.finished).await.result, Err(FailureKind::Cancelled));
                assert!(started.response.cleanup.clone().now_or_never().is_none());
                drop(bytes);
            } else {
                assert_eq!(item, Err(FailureKind::BodyTooLarge));
                assert_eq!(finished(started.finished).await.result, Err(FailureKind::BodyTooLarge));
            }
            cleanup(started.response.cleanup).await;
            wait_count(&server.shared.peer_eof, 1).await;
            f.finish().await;
        });
        checked(&server.finish(), 1, 0);
    }
}

#[test]
fn raw_stream_unpolled_queued_headers_expire_independently() {
    let mut reply = Reply::json(json!(null)); reply.gate_body = true;
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = start(&f, Instant::now() + Duration::from_millis(500), limits()).await;
        wait_count(&server.shared.headers, 1).await;
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::Deadline));
        assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::Deadline);
        cleanup(started.response.cleanup).await;
        wait_count(&server.shared.peer_eof, 1).await;
        f.finish().await;
    });
    checked(&server.finish(), 1, 0);
}

#[test]
fn raw_stream_failed_native_cleanup_quarantines_until_shutdown() {
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0,
        Reply::json(json!({"cleanup":"lost"})))]);
    let (capacity, supervisor) = run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = hooked_stream(&f, deadline(), TestHooks {
            lose_native_closed: true, ..TestHooks::default()
        });
        let mut head = started.response.result.await.unwrap();
        loop {
            match head.body.next().await.expect("failed cleanup must not become EOF") {
                Ok(_) => {},
                Err(kind) => { assert_eq!(kind, FailureKind::Cleanup); break; }
            }
        }
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::Cleanup));
        assert_eq!(started.response.cleanup.await, Err(FailureKind::Cleanup));
        let capacity = f.capacity.clone(); let supervisor = f.client.supervisor_for_test();
        drop(head); drop(f);
        assert_eq!(capacity.available_permits(), 0);
        assert!(!supervisor.is_finished());
        (capacity, supervisor)
    });
    assert_eq!(capacity.available_permits(), 1);
    assert!(supervisor.is_finished());
    checked(&server.finish(), 1, 1);
}

#[test]
fn raw_stream_default_provider_remains_unsupported() {
    struct Existing(Option<tokio::sync::OwnedSemaphorePermit>);
    impl OwnedInvocation for Existing {
        fn raw(self: Box<Self>, _: Value) -> InvocationRun<port::RawResult> { panic!("not called") }
        fn messages(self: Box<Self>, _: Value)
            -> InvocationRun<port::Completed<framer_core::convert::AssembledResponse>> { panic!("not called") }
        fn responses(self: Box<Self>, _: Value)
            -> InvocationRun<port::Completed<framer_core::convert::AssembledResponses>> { panic!("not called") }
    }
    run(async {
        let capacity = Arc::new(Semaphore::new(1));
        let provider: Box<dyn OwnedInvocation> = Box::new(Existing(Some(capacity.clone().try_acquire_owned().unwrap())));
        let started = provider.raw_stream(stream_payload(), limits());
        assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::UnsupportedStreaming);
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::UnsupportedStreaming));
        cleanup(started.response.cleanup).await;
        assert_eq!(capacity.available_permits(), 1);
    });
}

#[test]
fn raw_stream_real_owner_tail_delays_eof_and_capacity_release() {
    let bytes = b"tail-gated body".to_vec();
    let mut reply = Reply::json(json!(null)); reply.body = bytes.clone();
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let (release, tail) = oneshot::channel();
        let started = hooked_stream(&f, deadline(), TestHooks { tail_gate: Some(tail), ..TestHooks::default() });
        let mut head = started.response.result.await.unwrap();
        let mut received = Vec::new();
        while received.len() < bytes.len() {
            received.extend_from_slice(&head.body.next().await.unwrap().unwrap());
        }
        assert_eq!(received, bytes);
        assert!(tokio::time::timeout(PAUSE, head.body.next()).await.is_err());
        assert!(started.finished.clone().now_or_never().is_none());
        assert!(started.response.cleanup.clone().now_or_never().is_none());
        assert_eq!(f.capacity.available_permits(), 0);
        release.send(()).unwrap();
        assert!(head.body.next().await.is_none());
        assert_eq!(finished(started.finished).await.result, Ok(()));
        cleanup(started.response.cleanup).await;
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn raw_stream_body_drop_keeps_capacity_until_real_owner_tail() {
    let mut reply = Reply::json(json!(null)); reply.gate_body = true;
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let (release, tail) = oneshot::channel();
        let started = hooked_stream(&f, deadline(), TestHooks { tail_gate: Some(tail), ..TestHooks::default() });
        drop(started.response.result.await.unwrap());
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::Cancelled));
        assert!(started.response.cleanup.clone().now_or_never().is_none());
        assert_eq!(f.capacity.available_permits(), 0);
        release.send(()).unwrap();
        cleanup(started.response.cleanup).await;
        wait_count(&server.shared.peer_eof, 1).await;
        f.finish().await;
    });
    checked(&server.finish(), 1, 0);
}

#[test]
fn raw_stream_chunk_and_total_limits_end_with_typed_error() {
    for bound in [RawStreamLimits { chunk_bytes: 1, ..limits() },
        RawStreamLimits { total_bytes: 7, ..limits() }] {
        let mut reply = Reply::json(json!(null)); reply.body = b"abcdefghijklmnopqrstuvwxyz".to_vec();
        let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let started = start(&f, deadline(), bound).await;
            let mut head = started.response.result.await.unwrap();
            let mut emitted = 0;
            loop {
                match head.body.next().await.expect("explicit over-limit error") {
                    Ok(bytes) => { assert!(bytes.len() <= bound.chunk_bytes); emitted += bytes.len(); }
                    Err(kind) => { assert_eq!(kind, FailureKind::BodyTooLarge); break; }
                }
            }
            assert!(head.body.next().await.is_none());
            assert!(emitted <= bound.total_bytes);
            let done = finished(started.finished).await;
            assert_eq!(done.result, Err(FailureKind::BodyTooLarge));
            assert_eq!(done.emitted_bytes, emitted);
            cleanup(started.response.cleanup).await;
            f.finish().await;
        });
        let snapshot = server.finish();
        assert_eq!(snapshot.fault, None); assert_eq!(snapshot.requests, 1);
    }
}

#[test]
fn raw_stream_header_limit_rejects_before_handoff() {
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0,
        Reply::json(json!({"fixture":true})))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = start(&f, deadline(), RawStreamLimits { header_bytes: 1, ..limits() }).await;
        assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::BodyTooLarge);
        let done = finished(started.finished).await;
        assert_eq!(done.result, Err(FailureKind::BodyTooLarge));
        assert!(done.headers.is_none()); assert_eq!(done.emitted_bytes, 0);
        cleanup(started.response.cleanup).await;
        f.finish().await;
    });
    let snapshot = server.finish();
    assert_eq!(snapshot.fault, None); assert_eq!(snapshot.requests, 1);
}

#[test]
fn raw_stream_refusals_keep_original_status_headers_and_body() {
    for status in [403, 429] {
        let mut reply = Reply::json(json!({"refused":true})); reply.status = status;
        let bytes = reply.body.clone();
        let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let started = start(&f, deadline(), limits()).await;
            let head = started.response.result.await.unwrap();
            let headers = head.headers.clone();
            assert_eq!(headers.status, status);
            assert_eq!(headers.usage_header.as_deref(), Some("remaining=7.5&fixture=application"));
            assert_eq!(drain(head.body).await, bytes);
            let done = finished(started.finished).await;
            assert!(Arc::ptr_eq(&headers, &done.headers.unwrap()));
            assert_eq!(done.result, Ok(())); // Byte delivery, not upstream success.
            cleanup(started.response.cleanup).await;
            assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
            f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

#[test]
fn raw_stream_invalid_limits_and_payloads_fail_before_authorization() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let base = limits();
        for bound in [RawStreamLimits { chunk_bytes: 0, ..base },
            RawStreamLimits { retained_bytes: 1, ..base },
            RawStreamLimits { total_bytes: 1, ..base },
            RawStreamLimits { retained_bytes: usize::MAX, ..base },
            RawStreamLimits { total_bytes: usize::MAX, ..base },
            RawStreamLimits { header_bytes: 0, ..base }] {
            let started = start(&f, deadline(), bound).await;
            assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::InvalidRequest);
            assert_eq!(finished(started.finished).await.result, Err(FailureKind::InvalidRequest));
            cleanup(started.response.cleanup).await;
        }
        for payload in [json!(null), json!({"stream":"true"})] {
            let call = f.admit(Route::Raw(ClientDialect::OpenAiResponses), deadline()).await.unwrap();
            let started = call.raw_stream(payload, limits());
            assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::InvalidRequest);
            cleanup(started.response.cleanup).await;
        }
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn raw_stream_deadline_wakes_credit_blocked_body_and_preserves_credit() {
    let mut reply = Reply::json(json!(null)); reply.body = b"abcdefghijklmnop".to_vec();
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let mut bound = limits(); bound.retained_bytes = bound.chunk_bytes;
        let started = start(&f, Instant::now() + Duration::from_millis(500), bound).await;
        let mut head = started.response.result.await.unwrap();
        let bytes = head.body.next().await.unwrap().unwrap();
        assert!(tokio::time::timeout(PAUSE, head.body.next()).await.is_err());
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::Deadline));
        assert_eq!(head.body.next().await, Some(Err(FailureKind::Deadline)));
        assert!(started.response.cleanup.clone().now_or_never().is_none());
        assert_eq!(f.capacity.available_permits(), 0);
        drop(bytes);
        cleanup(started.response.cleanup).await;
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn raw_stream_aliases_preserve_non_sse_bytes_and_missing_headers() {
    for (dialect, path, name) in [
        (ClientDialect::AnthropicMessages, "/ai/v3/anthropic/messages/", "AnthropicMessages"),
        (ClientDialect::OpenAiChat, "/ai/v3/chat/", "OpenRouterChat"),
        (ClientDialect::OpenAiResponses, "/ai/v3/openai/responses/", "OpenAiResponses"),
    ] {
        let bytes = vec![0, 255, 13, 10, 240, 159, 153, 130, 42];
        let mut reply = Reply::json(json!(null)); reply.body = bytes.clone();
        reply.content_type = None; reply.usage = None; reply.status = 207;
        let server = Server::new(vec![expected(path, stream_payload(), 0, reply)]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let until = deadline();
            let started = f.admit(Route::Raw(dialect), until).await.unwrap()
                .raw_stream(stream_payload(), limits());
            let head = started.response.result.await.unwrap();
            assert_eq!(head.headers.status, 207);
            assert!(head.headers.usage_header.is_none());
            assert!(head.headers.original_content_type.is_none());
            assert_eq!(head.headers.content_type.as_ref(), MISSING_TYPE);
            assert_eq!(drain(head.body).await, bytes);
            assert_eq!(finished(started.finished).await.result, Ok(()));
            cleanup(started.response.cleanup).await;
            seen(&f.supplier, &f.scope, until, name, &[stream_payload()]);
            f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

#[test]
fn raw_stream_byte_credits_follow_clones_and_slices() {
    let bytes = b"abcdefghijklmnopqrstuvwxyz".to_vec();
    let mut reply = Reply::json(json!(null)); reply.body = bytes.clone();
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let mut bound = limits(); bound.retained_bytes = bound.chunk_bytes;
        let started = start(&f, deadline(), bound).await;
        let mut head = started.response.result.await.unwrap();
        let first = head.body.next().await.unwrap().unwrap();
        let mut received = first.to_vec();
        let cloned = first.clone(); let sliced = first.slice(..1); drop(first);
        assert!(tokio::time::timeout(PAUSE, head.body.next()).await.is_err());
        drop(cloned);
        assert!(tokio::time::timeout(PAUSE, head.body.next()).await.is_err());
        drop(sliced);
        received.extend(drain(head.body).await);
        assert_eq!(received, bytes);
        assert_eq!(finished(started.finished).await.result, Ok(()));
        cleanup(started.response.cleanup).await;
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn raw_stream_cleanup_retains_capacity_for_bytes_after_eof() {
    let mut reply = Reply::json(json!(null)); reply.body = b"abc".to_vec();
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let mut bound = limits(); bound.retained_bytes = 20;
        let started = start(&f, deadline(), bound).await;
        let progress = started.response.completed_attempts.unwrap();
        let mut head = started.response.result.await.unwrap();
        let mut retained = Vec::new();
        while let Some(chunk) = head.body.next().await { retained.push(chunk.unwrap()); }
        assert_eq!(retained.iter().map(|b| b.len()).sum::<usize>(), 3);
        assert_eq!(finished(started.finished).await.result, Ok(()));
        assert!(started.response.cleanup.clone().now_or_never().is_none());
        assert_eq!(f.capacity.available_permits(), 0);
        let snapshot = progress(); assert_eq!(snapshot.len(), 1);
        tokio::time::sleep(PAUSE).await;
        assert_eq!(progress()[0].elapsed, snapshot[0].elapsed);
        drop(retained);
        cleanup(started.response.cleanup).await;
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn raw_stream_deadline_revokes_never_polled_body() {
    let mut reply = Reply::json(json!({"held":true})); reply.gate_body = true;
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = start(&f, Instant::now() + Duration::from_millis(500), limits()).await;
        let mut head = started.response.result.await.unwrap();
        let done = finished(started.finished).await;
        assert_eq!(done.result, Err(FailureKind::Deadline));
        assert_eq!(done.headers.unwrap().status, 200);
        cleanup(started.response.cleanup).await;
        assert_eq!(f.capacity.available_permits(), 1);
        assert_eq!(head.body.next().await, Some(Err(FailureKind::Deadline)));
        wait_count(&server.shared.peer_eof, 1).await;
        f.finish().await;
    });
    checked(&server.finish(), 1, 0);
}

#[test]
fn raw_stream_deadline_before_headers_has_no_success_handoff() {
    let server = Server::with_held_headers(vec![expected("/ai/v3/openai/responses/",
        stream_payload(), 0, Reply::json(json!(null)))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = start(&f, Instant::now() + Duration::from_millis(500), limits()).await;
        let error = started.response.result.await.err().unwrap();
        assert_eq!(error.kind, FailureKind::Deadline);
        let done = finished(started.finished).await;
        assert_eq!(done.result, Err(FailureKind::Deadline));
        assert!(done.headers.is_none()); assert_eq!(done.emitted_bytes, 0);
        cleanup(started.response.cleanup).await;
        wait_count(&server.shared.peer_eof, 1).await;
        f.finish().await;
    });
    let snapshot = server.finish();
    assert_eq!(snapshot.fault, None); assert_eq!(snapshot.requests, 1);
    assert_eq!(snapshot.headers, 0); assert_eq!(snapshot.replies, 0);
}

#[test]
fn raw_stream_dropping_body_cancels_without_polling_it() {
    let mut reply = Reply::json(json!({"held":true})); reply.gate_body = true;
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = start(&f, deadline(), limits()).await;
        let head = started.response.result.await.unwrap();
        drop(head);
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::Cancelled));
        cleanup(started.response.cleanup).await;
        wait_count(&server.shared.peer_eof, 1).await;
        f.finish().await;
    });
    checked(&server.finish(), 1, 0);
}

#[test]
fn raw_stream_dropped_header_future_stops_before_authorization() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = start(&f, deadline(), limits()).await;
        drop(started.response.result);
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::Cancelled));
        cleanup(started.response.cleanup).await;
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn raw_stream_authorization_obeys_original_deadline() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier { never: true, ..Supplier::default() }, policy());
        let started = start(&f, Instant::now() + Duration::from_millis(500), limits()).await;
        let error = started.response.result.await.err().unwrap();
        assert_eq!(error.kind, FailureKind::Deadline);
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::Deadline));
        cleanup(started.response.cleanup).await;
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        assert_eq!(f.supplier.cancelled.load(Ordering::Acquire), 1);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn raw_stream_cancel_revokes_never_polled_body() {
    let mut reply = Reply::json(json!({"held":true})); reply.gate_body = true;
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = start(&f, deadline(), limits()).await;
        let mut head = started.response.result.await.unwrap();
        started.response.cancellation.cancel();
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::Cancelled));
        cleanup(started.response.cleanup).await;
        assert_eq!(f.capacity.available_permits(), 1);
        assert_eq!(head.body.next().await, Some(Err(FailureKind::Cancelled)));
        assert_eq!(head.body.next().await, None);
        wait_count(&server.shared.peer_eof, 1).await;
        f.finish().await;
    });
    checked(&server.finish(), 1, 0);
}

#[test]
fn raw_stream_real_headers_precede_body_and_preserve_fragmented_bytes() {
    let bytes = "event: delta\r\ndata: {\"text\":\"λ🙂\"}\r\n\r\ndata: [DONE]\n\n".as_bytes().to_vec();
    let mut reply = Reply::json(json!(null));
    reply.body = bytes.clone(); reply.content_type = Some("text/event-stream");
    let server = Server::with_held_headers(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = start(&f, deadline(), limits()).await;
        let mut head = started.response.result;
        wait_count(&server.shared.requests, 1).await;
        assert!(tokio::time::timeout(PAUSE, &mut head).await.is_err());
        assert_eq!(server.shared.headers.load(Ordering::Acquire), 0);
        server.shared.release_headers.store(true, Ordering::Release);
        let head = tokio::time::timeout(STEP, head).await.unwrap().unwrap();
        assert_eq!(head.headers.status, 200);
        assert_eq!(head.headers.original_content_type.as_deref(), Some("text/event-stream"));
        assert_eq!(head.headers.content_type.as_ref(), "text/event-stream");
        assert!(started.response.cleanup.clone().now_or_never().is_none());
        assert_eq!(drain(head.body).await, bytes);
        let done = finished(started.finished).await;
        assert_eq!(done.result, Ok(())); assert_eq!(done.emitted_bytes, bytes.len());
        cleanup(started.response.cleanup).await;
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}
