include!("raw_stream_fixture.rs");

#[test]
fn raw_stream_lost_worker_without_native_registration_quarantines_capacity() {
    let server = Server::new(vec![]);
    let (capacity, supervisor) = run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = super::raw_stream::launch_for_test(call_for_hook(&f, deadline()),
            stream_payload(), limits(), Box::new(|_, _| {
                panic!("synthetic raw worker loss before native registration")
            }));
        assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::Internal);
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::Internal));
        assert_eq!(started.response.cleanup.await, Err(FailureKind::Cleanup));
        let capacity = f.capacity.clone(); let supervisor = f.client.supervisor_for_test();
        drop(f);
        assert_eq!(capacity.available_permits(), 0);
        assert!(!supervisor.is_finished());
        (capacity, supervisor)
    });
    assert_eq!(capacity.available_permits(), 1);
    assert!(supervisor.is_finished());
    checked(&server.finish(), 0, 0);
}

#[test]
fn raw_stream_scope_closure_revokes_never_polled_body() {
    let mut reply = Reply::json(json!(null)); reply.gate_body = true;
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = start(&f, deadline(), limits()).await;
        let mut head = started.response.result.await.unwrap();
        close(&f.client, &f.scope).await;
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::ScopeClosed));
        cleanup(started.response.cleanup).await;
        assert_eq!(f.capacity.available_permits(), 1);
        assert_eq!(head.body.next().await, Some(Err(FailureKind::ScopeClosed)));
        assert_eq!(head.body.next().await, None);
        wait_count(&server.shared.peer_eof, 1).await;
        f.finish().await;
    });
    checked(&server.finish(), 1, 0);
}

#[test]
fn raw_stream_known_pre_admission_failure_releases_without_closing_scope() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = super::raw_stream::launch_for_test(call_for_hook(&f, deadline()),
            stream_payload(), limits(), Box::new(move |parts, payload| {
                pending_with_start(parts, payload, |_, _, _, _| {
                    Err(bridge::StartError::Admission(AdmissionError::Full))
                }).map_err(ApplicationError::Start)
            }));
        assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::Overloaded);
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::Overloaded));
        cleanup(started.response.cleanup).await;
        assert_eq!(f.capacity.available_permits(), 1);
        assert!(f.scope.closed().now_or_never().is_none());
        // The unchanged generation can still admit a new invocation.
        drop(f.admit(Route::Raw(ClientDialect::OpenAiResponses), deadline()).await.unwrap());
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn raw_stream_unconfirmed_spawn_failure_quarantines_capacity() {
    let server = Server::new(vec![]);
    let (capacity, supervisor) = run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = super::raw_stream::launch_for_test(call_for_hook(&f, deadline()),
            stream_payload(), limits(), Box::new(move |parts, payload| {
                // Inject SpawnFailed through the real account admission seam.
                // This does not claim to reproduce a Tokio spawning panic.
                pending_with_start(parts, payload, |_, _, _, _| {
                    Err(bridge::StartError::Admission(AdmissionError::SpawnFailed))
                }).map_err(ApplicationError::Start)
            }));
        assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::Transport);
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::Transport));
        assert!(f.scope.closed().await.is_err());
        assert_eq!(started.response.cleanup.await, Err(FailureKind::Cleanup));
        let capacity = f.capacity.clone(); let supervisor = f.client.supervisor_for_test();
        drop(f);
        assert_eq!(capacity.available_permits(), 0);
        assert!(!supervisor.is_finished());
        (capacity, supervisor)
    });
    assert_eq!(capacity.available_permits(), 1);
    assert!(supervisor.is_finished());
    checked(&server.finish(), 0, 0);
}

#[test]
fn raw_stream_attachment_failure_quarantines_even_if_scope_reports_error() {
    for missing_completion in [false, true] {
        let server = Server::new(vec![]);
        let (capacity, supervisor) = run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let (release, tail) = oneshot::channel();
            let started = super::raw_stream::launch_for_test(call_for_hook(&f, deadline()),
                stream_payload(), limits(), Box::new(move |parts, payload| {
                    pending_with_start(parts, payload, |pool, inputs, until, content_type| {
                        let mut owner = pool.start_for_test(inputs, Mode::Raw, until,
                            TestHooks { tail_gate: Some(tail), ..TestHooks::default() })
                            .map_err(bridge::StartError::Admission)?;
                        drop(owner.take_response());
                        if missing_completion { drop(owner.take_completion()); }
                        bridge::from_owner(owner, content_type).map_err(bridge::StartError::Attach)
                    }).map_err(ApplicationError::Start)
                }));
            assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::Transport);
            assert_eq!(finished(started.finished).await.result, Err(FailureKind::Transport));
            assert!(f.scope.closed().await.is_err());
            assert_eq!(started.response.cleanup.await, Err(FailureKind::Cleanup));
            let capacity = f.capacity.clone();
            let supervisor = f.client.supervisor_for_test();
            assert_eq!(capacity.available_permits(), 0);
            release.send(()).unwrap();
            drop(f);
            assert!(!supervisor.is_finished());
            (capacity, supervisor)
        });
        assert_eq!(capacity.available_permits(), 1);
        assert!(supervisor.is_finished());
        checked(&server.finish(), 0, 0);
    }
}

#[test]
fn raw_stream_raced_start_waits_for_successful_account_closure() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let (release, tail) = oneshot::channel();
        let started = super::raw_stream::launch_for_test(call_for_hook(&f, deadline()),
            stream_payload(), limits(), Box::new(move |parts, payload| {
                pending_with_start(parts, payload, |pool, inputs, until, content_type| {
                    let owner = pool.start_for_test(inputs, Mode::Raw, until,
                        TestHooks { tail_gate: Some(tail), ..TestHooks::default() })
                        .map_err(bridge::StartError::Admission)?;
                    // Work exists, but start_with has not registered it yet.
                    let _closed = parts.native.close_account(&parts.scope).unwrap();
                    bridge::from_owner(owner, content_type).map_err(bridge::StartError::Attach)
                }).map_err(ApplicationError::Start)
            }));
        assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::ScopeClosed);
        assert_eq!(finished(started.finished).await.result, Err(FailureKind::ScopeClosed));
        assert!(started.response.cleanup.clone().now_or_never().is_none());
        assert!(f.scope.closed().now_or_never().is_none());
        assert_eq!(f.capacity.available_permits(), 0);
        release.send(()).unwrap();
        cleanup(started.response.cleanup).await;
        assert_eq!(f.scope.closed().await, Ok(()));
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}
