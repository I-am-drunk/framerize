include!("fixture.rs");

use super::TrackedAuthorization;
use crate::experimental_native::application::{ApplicationError, AttemptMetadata as NativeMetadata};
use crate::experimental_native::native_account::CloseError;
use crate::experimental_native::native_owner::{AdmissionError, RequestPool};

fn start_context() -> RequestContext {
    RequestContext {
        dialect: Dialect::OpenAiResponses, project: project(), payload: responses_payload(),
        feature_category: "fixture-application-category".into(),
        feature_name: "fixture-application-feature".into(), step: "fixture-application-step".into(),
        chat_session: CHAT.into(), step_index: 0, turn_id: "fixture-application-turn-0".into(),
        user_prompt: "fixture outer annotation\nkept exactly".into(),
    }
}

fn raced_start(client: &NativeAccountClient, scope: &AccountScope, until: Instant,
    tail: oneshot::Receiver<()>) -> Result<bridge::Pending, ClientError> {
    client.start_for_test(scope, start_context(), auth(scope), until, MISSING_TYPE.into(),
        |pool, inputs, deadline, content_type| {
            let owner = pool.start_for_test(inputs, Mode::Raw, deadline,
                TestHooks { tail_gate: Some(tail), ..TestHooks::default() })
                .map_err(bridge::StartError::Admission)?;
            // Close after real request admission, before account registration.
            drop(client.close_account(scope).expect("raced account close"));
            bridge::from_owner(owner, content_type).map_err(bridge::StartError::Attach)
        })
}

async fn wait_supervisor(supervisor: tokio::task::AbortHandle) {
    tokio::time::timeout(STEP, async {
        while !supervisor.is_finished() { tokio::time::sleep(TICK).await; }
    }).await.expect("supervisor released after confirmed cleanup");
}

fn previous(index: u32) -> NativeMetadata {
    NativeMetadata { status: 200, usage_header: Some(format!("fixture-{index}")),
        original_content_type: Some("application/json".into()), step_index: index,
        elapsed: Duration::from_millis(10) }
}

async fn draining_case(server: &Server, nested: bool, drop_result: bool) {
    let f = Fixture::new(server, Supplier::default(), policy());
    let client = f.client.clone(); let scope = f.scope.clone();
    let supplier = f.supplier.clone(); let until = deadline();
    let (release, tail) = oneshot::channel();
    let (mapped_tx, mapped_rx) = oneshot::channel();
    let permit = f.capacity.clone().try_acquire_owned().unwrap();
    let started = super::lifetime::launch(f.client.clone(), f.scope.clone(), permit, until, 1,
        move |registry| async move {
            let tracked = TrackedAuthorization { inner: supplier, registry };
            let error = match raced_start(&client, &scope, until, tail) {
                Err(ClientError::Draining) => ApplicationError::Start(ClientError::Draining),
                _ => panic!("expected a real post-admission draining error"),
            };
            let error = if nested {
                ApplicationError::Chain {
                    error: Box::new(ApplicationError::Chain {
                        error: Box::new(error), completed_attempts: vec![previous(1)],
                    }), completed_attempts: vec![previous(0)],
                }
            } else { error };
            let error = tracked.error(error);
            mapped_tx.send(()).unwrap();
            Err::<(), _>(error)
        });
    tokio::time::timeout(STEP, mapped_rx).await.unwrap().unwrap();
    let capacity = f.capacity.clone(); let supervisor = f.client.supervisor_for_test();
    drop(f);
    if drop_result { drop(started.result); }
    else {
        let error = tokio::time::timeout(STEP, started.result).await.unwrap().err().unwrap();
        assert_eq!(error.kind, FailureKind::ScopeClosed);
        assert_eq!(error.attempts.len(), if nested { 2 } else { 0 });
        for (index, attempt) in error.attempts.iter().enumerate() {
            assert_eq!(attempt.step_index, index as u32);
            assert_eq!(attempt.usage_header, Some(format!("fixture-{index}")));
        }
    }
    assert!(started.cleanup.clone().now_or_never().is_none());
    assert_eq!(capacity.available_permits(), 0);
    assert!(!supervisor.is_finished());
    release.send(()).unwrap();
    assert_eq!(tokio::time::timeout(STEP, started.cleanup).await.unwrap(), Ok(()));
    assert_eq!(capacity.available_permits(), 1);
    wait_supervisor(supervisor).await;
}

#[test]
fn dispatch_start_raced_registration_waits_for_scope_close() {
    let server = Server::new(vec![]);
    run(draining_case(&server, false, false));
    checked(&server.finish(), 0, 0);
}

#[test]
fn dispatch_start_nested_chain_keeps_cleanup_and_metadata() {
    let server = Server::new(vec![]);
    run(draining_case(&server, true, false));
    checked(&server.finish(), 0, 0);
}

#[test]
fn dispatch_start_dropped_result_keeps_cleanup_capacity() {
    let server = Server::new(vec![]);
    run(draining_case(&server, false, true));
    checked(&server.finish(), 0, 0);
}

async fn pool_free(pool: &RequestPool) {
    tokio::time::timeout(STEP, async {
        loop {
            pool.reap();
            if pool.snapshot().occupied == 0 { break; }
            tokio::time::sleep(TICK).await;
        }
    }).await.expect("real native driver cleanup");
    assert_eq!(pool.snapshot().quarantined, 0);
}

fn unconfirmed_case(spawn_failed: bool) {
    let server = Server::new(vec![]);
    let (capacity, supervisor) = run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let client = f.client.clone(); let scope = f.scope.clone();
        let supplier = f.supplier.clone(); let until = deadline();
        let closed = scope.closed();
        let (release, tail) = oneshot::channel();
        let (pool_tx, pool_rx) = oneshot::channel();
        let permit = f.capacity.clone().try_acquire_owned().unwrap();
        let started = super::lifetime::launch(f.client.clone(), f.scope.clone(), permit, until, 1,
            move |registry| async move {
                let tracked = TrackedAuthorization { inner: supplier, registry };
                let result = client.start_for_test(&scope, start_context(), auth(&scope), until,
                    MISSING_TYPE.into(), |pool, inputs, deadline, content_type| {
                        assert!(pool_tx.send(pool.clone()).is_ok());
                        if spawn_failed {
                            // Exercise the real account registration branch, without
                            // claiming that the OS actually failed to spawn a task.
                            return Err(bridge::StartError::Admission(AdmissionError::SpawnFailed));
                        }
                        let mut owner = pool.start_for_test(inputs, Mode::Raw, deadline,
                            TestHooks { tail_gate: Some(tail), ..TestHooks::default() })
                            .map_err(bridge::StartError::Admission)?;
                        drop(owner.take_response());
                        bridge::from_owner(owner, content_type).map_err(bridge::StartError::Attach)
                    });
                let error = result.err().expect("start must return no Pending");
                assert!(matches!((&error, spawn_failed),
                    (ClientError::Admission(AdmissionError::SpawnFailed), true)
                    | (ClientError::Attachment, false)));
                Err::<(), _>(tracked.error(ApplicationError::Start(error)))
            });
        let capacity = f.capacity.clone(); let supervisor = f.client.supervisor_for_test();
        drop(f);
        let error = tokio::time::timeout(STEP, started.result).await.unwrap().err().unwrap();
        assert_eq!(error.kind, FailureKind::Transport);
        assert_eq!(tokio::time::timeout(STEP, closed).await.unwrap(),
            Err(if spawn_failed { CloseError::StartUnconfirmed } else { CloseError::Attachment }));
        assert_eq!(tokio::time::timeout(STEP, started.cleanup).await.unwrap(), Err(FailureKind::Cleanup));
        assert_eq!(capacity.available_permits(), 0);
        if !spawn_failed { release.send(()).unwrap(); } else { drop(release); }
        let pool = pool_rx.await.unwrap();
        pool_free(&pool).await;
        assert_eq!(capacity.available_permits(), 0);
        assert!(!supervisor.is_finished());
        (capacity, supervisor)
    });
    assert_eq!(capacity.available_permits(), 1);
    assert!(supervisor.is_finished());
    checked(&server.finish(), 0, 0);
}

#[test]
fn dispatch_start_attachment_keeps_capacity_after_driver_exit() { unconfirmed_case(false); }

#[test]
fn dispatch_start_spawn_failed_registration_quarantines_capacity() { unconfirmed_case(true); }

#[test]
fn dispatch_start_known_rejections_release_without_closing_scope() {
    for invalid_context in [false, true] {
        let server = Server::new(vec![]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let client = f.client.clone(); let scope = f.scope.clone();
            let supplier = f.supplier.clone(); let until = deadline();
            let permit = f.capacity.clone().try_acquire_owned().unwrap();
            let started = super::lifetime::launch(f.client.clone(), f.scope.clone(), permit, until, 1,
                move |registry| async move {
                    let tracked = TrackedAuthorization { inner: supplier, registry };
                    let mut context = start_context();
                    if invalid_context { context.turn_id.clear(); }
                    let result = client.start_for_test(&scope, context, auth(&scope), until,
                        MISSING_TYPE.into(), |_, _, _, _| {
                            assert!(!invalid_context, "invalid context must stop before admission");
                            Err(bridge::StartError::Admission(AdmissionError::Full))
                        });
                    Err::<(), _>(tracked.error(ApplicationError::Start(result.err().unwrap())))
                });
            let error = completed(started).await.err().unwrap();
            assert_eq!(error.kind, if invalid_context { FailureKind::MissingContext }
                else { FailureKind::Overloaded });
            assert_eq!(f.capacity.available_permits(), 1);
            assert!(f.scope.closed().now_or_never().is_none());
            f.finish().await;
        });
        checked(&server.finish(), 0, 0);
    }
}

fn worker_loss_case(after_observer: bool) {
    let server = Server::new(vec![]);
    let (capacity, supervisor) = run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let client = f.client.clone(); let scope = f.scope.clone();
        let until = deadline(); let closed = scope.closed();
        let (release, tail) = oneshot::channel();
        let (pool_tx, pool_rx) = oneshot::channel();
        let observed = Arc::new(AtomicUsize::new(0));
        let worker_observed = observed.clone();
        let permit = f.capacity.clone().try_acquire_owned().unwrap();
        let started = super::lifetime::launch(f.client.clone(), f.scope.clone(), permit, until, 1,
            move |registry| async move {
                let pending = client.start_for_test(&scope, start_context(), auth(&scope), until,
                    MISSING_TYPE.into(), |pool, inputs, deadline, content_type| {
                        let owner = pool.start_for_test(inputs, Mode::Raw, deadline,
                            TestHooks { tail_gate: Some(tail), ..TestHooks::default() }).unwrap();
                        assert!(pool_tx.send(pool.clone()).is_ok());
                        if !after_observer {
                            panic!("fixture worker loss after native admission, before observer");
                        }
                        bridge::from_owner(owner, content_type).map_err(bridge::StartError::Attach)
                    }).unwrap();
                registry.started(pending.cleanup.clone(), pending.cancellation.clone());
                worker_observed.store(1, Ordering::Release);
                drop(pending);
                panic!("fixture worker loss after observer");
                #[allow(unreachable_code)]
                Ok::<(), DispatchError>(())
            });
        let capacity = f.capacity.clone(); let supervisor = f.client.supervisor_for_test();
        drop(f);
        let error = tokio::time::timeout(STEP, started.result).await.unwrap().err().unwrap();
        assert_eq!(error.kind, FailureKind::Internal);
        assert_eq!(observed.load(Ordering::Acquire), usize::from(after_observer));
        assert_eq!(capacity.available_permits(), 0);
        if after_observer {
            assert!(started.cleanup.clone().now_or_never().is_none());
            assert!(closed.clone().now_or_never().is_none());
        } else {
            assert_eq!(tokio::time::timeout(STEP, started.cleanup.clone()).await.unwrap(),
                Err(FailureKind::Cleanup));
            assert_eq!(tokio::time::timeout(STEP, closed.clone()).await.unwrap(),
                Err(CloseError::StartUnconfirmed));
        }
        release.send(()).unwrap();
        let pool = pool_rx.await.unwrap();
        pool_free(&pool).await;
        assert_eq!(tokio::time::timeout(STEP, started.cleanup).await.unwrap(), Err(FailureKind::Cleanup));
        assert_eq!(capacity.available_permits(), 0);
        assert!(!supervisor.is_finished());
        (capacity, supervisor)
    });
    assert_eq!(capacity.available_permits(), 1);
    assert!(supervisor.is_finished());
    checked(&server.finish(), 0, 0);
}

#[test]
fn dispatch_start_worker_loss_before_observer_keeps_capacity() { worker_loss_case(false); }

#[test]
fn dispatch_start_worker_loss_after_observer_waits_then_quarantines() { worker_loss_case(true); }
