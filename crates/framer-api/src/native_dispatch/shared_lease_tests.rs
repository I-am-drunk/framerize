// Synthetic accounts, loopback peer and existing cleanup gates only.
include!("composed_fixture.rs");
#[path = "pool_lease_tests.rs"]
mod pool_tests;
use framer_core::account_lease::{AccountLeaseCoordinator, ChangeResult, LeaseError, LeaseLimits};
use crate::experimental_native::{authorization::{ContextPolicy, CredentialMaterial,
    CredentialSource, GenerationAuthorization}, native_account::{PreparationControl,
    PreparationFailure, PreparationResult}};

struct HeldSource { entered: Semaphore, release: Semaphore, unconfirmed: bool }
impl CredentialSource for HeldSource {
    fn issue<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        _: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async move {
            assert_eq!(scope.active_user(), &session.active_user);
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            if self.unconfirmed { PreparationResult::unconfirmed() }
            else { PreparationResult::finished(Err(PreparationFailure::Cancelled)) }
        }.boxed()
    }
}

fn install_held_source(f: &mut Fixture, unconfirmed: bool) -> Arc<HeldSource> {
    let source = Arc::new(HeldSource { entered: Semaphore::new(0),
        release: Semaphore::new(0), unconfirmed });
    let authorization = GenerationAuthorization::new(f.client.clone(), f.scope.clone(),
        source.clone(), ContextPolicy { project: project(),
            feature_category: "fixture-application-category".into(),
            feature_name: "fixture-application-feature".into(),
            step: "fixture-application-step".into(), display_prompt_chars: 64 }).unwrap();
    f.provider.bindings.get_mut(&f.scope.account()).unwrap().authorization = Some(Arc::new(authorization));
    source
}

fn leases() -> AccountLeaseCoordinator {
    AccountLeaseCoordinator::new(LeaseLimits { accounts: 4, users: 4, waiters: 4,
        user_id_bytes: 128, total_user_bytes: 512 }).unwrap()
}

async fn attach(mut fixture: Fixture, leases: &AccountLeaseCoordinator) -> Fixture {
    let mut change = leases.begin_change(fixture.scope.account()).unwrap().await.unwrap();
    change.fence_user(&fixture.scope.active_user().0).unwrap().await.unwrap();
    change.stage_binding(fixture.scope.account(), Some(&fixture.scope.active_user().0)).unwrap();
    change.finish(ChangeResult::Completed).unwrap();
    fixture.provider = fixture.provider.with_account_leases(leases.clone());
    fixture
}

fn assert_busy(leases: &AccountLeaseCoordinator, fixture: &Fixture) {
    assert!(leases.operation(fixture.scope.account(), &fixture.scope.active_user().0)
        .unwrap().now_or_never().is_none());
    assert_eq!(leases.stats().unwrap().waiters, 0);
}

fn assert_reusable(leases: &AccountLeaseCoordinator, fixture: &Fixture) {
    leases.operation(fixture.scope.account(), &fixture.scope.active_user().0)
        .unwrap().now_or_never().expect("immediate shared admission").unwrap()
        .complete().unwrap();
}

#[test]
fn lease_busy_prepare_is_nonwaiting_and_other_accounts_progress() {
    let server = Server::new(vec![]);
    run(async {
        let leases = leases();
        let first = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let other = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let legacy = leases.operation(first.scope.account(), &first.scope.active_user().0)
            .unwrap().now_or_never().unwrap().unwrap();
        for _ in 0..8 {
            let error = first.admit(Route::Responses, deadline()).now_or_never()
                .expect("prepare must not wait for the account").err().unwrap();
            assert_eq!(error.kind, FailureKind::Overloaded);
            assert_eq!(first.capacity.available_permits(), 1);
            assert_eq!(leases.stats().unwrap().waiters, 0);
        }
        let admitted = other.admit(Route::Responses, deadline()).await.unwrap();
        assert_eq!(leases.stats().unwrap().operations, 2);
        drop(admitted);
        legacy.complete().unwrap();
        assert_reusable(&leases, &first);
        assert_eq!(first.supplier.calls.load(Ordering::Acquire), 0);
        assert_eq!(other.supplier.calls.load(Ordering::Acquire), 0);
        first.finish().await; other.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn lease_full_queue_rejects_and_does_not_leak_workspace_capacity() {
    let server = Server::new(vec![]);
    run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let queued: Vec<_> = (0..4).map(|_| leases.operation(f.scope.account(),
            &f.scope.active_user().0).unwrap()).collect();
        let error = f.admit(Route::Responses, deadline()).now_or_never().unwrap().err().unwrap();
        assert_eq!(error.kind, FailureKind::Overloaded);
        assert_eq!(leases.stats().unwrap().waiters, 4);
        assert_eq!(f.capacity.available_permits(), 1);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        drop(queued); assert_reusable(&leases, &f); f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn lease_unpolled_and_expired_preparation_release_without_authorizing() {
    let server = Server::new(vec![]);
    run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let permit = f.capacity.clone().try_acquire_owned().unwrap();
        let pending = f.provider.prepare(f.request(Route::Responses, deadline()), permit);
        assert_eq!(f.capacity.available_permits(), 0);
        assert_eq!(leases.stats().unwrap().operations, 0);
        assert_eq!(leases.stats().unwrap().waiters, 0);
        drop(pending);
        assert_eq!(f.capacity.available_permits(), 1);
        let error = f.admit(Route::Responses, Instant::now()).now_or_never().unwrap().err().unwrap();
        assert_eq!(error.kind, FailureKind::Deadline);
        assert_eq!(f.capacity.available_permits(), 1);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        assert_reusable(&leases, &f); f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn lease_dropping_an_unstarted_call_completes_prepared_ownership() {
    let server = Server::new(vec![]);
    run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let call = f.admit(Route::Responses, deadline()).await.unwrap();
        assert_busy(&leases, &f);
        assert_eq!(leases.stats().unwrap().operations, 1);
        assert_eq!(f.capacity.available_permits(), 0);
        drop(call);
        assert_eq!(f.capacity.available_permits(), 1);
        assert_eq!(leases.stats().unwrap().operations, 0);
        assert_reusable(&leases, &f);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn lease_invalid_stream_options_and_alias_drop_prepared_ownership() {
    let server = Server::new(vec![]);
    run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let mut invalid = limits(); invalid.chunk_bytes = 0;
        let raw = f.admit(Route::Raw(ClientDialect::OpenAiResponses), deadline()).await.unwrap()
            .raw_stream(stream_payload(), invalid);
        assert_eq!(leases.stats().unwrap().operations, 0);
        assert_eq!(raw.response.result.await.err().unwrap().kind, FailureKind::InvalidRequest);
        cleanup(raw.response.cleanup).await;
        for bad_alias in [false, true] {
            let mut requested = options(ClientDialect::OpenAiResponses);
            if !bad_alias { requested.limits.frame_bytes = 0; }
            let call = f.admit(Route::Responses, deadline()).await.unwrap();
            let composed = if bad_alias {
                call.composed_stream_for_client(responses_payload(), requested, String::new())
            } else { call.composed_stream(responses_payload(), requested) };
            assert_eq!(leases.stats().unwrap().operations, 0);
            assert_eq!(composed.response.result.await.err().unwrap().kind, FailureKind::InvalidRequest);
            cleanup(composed.response.cleanup).await;
        }
        assert_eq!(f.capacity.available_permits(), 1);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        assert_reusable(&leases, &f); f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn lease_closed_scope_fails_before_shared_admission_or_authorization() {
    let server = Server::new(vec![]);
    run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        close(&f.client, &f.scope).await;
        let error = f.admit(Route::Responses, deadline()).await.err().unwrap();
        assert_eq!(error.kind, FailureKind::ScopeClosed);
        assert_eq!(leases.stats().unwrap().operations, 0);
        assert_eq!(leases.stats().unwrap().waiters, 0);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        assert_eq!(f.capacity.available_permits(), 1);
        assert_reusable(&leases, &f); f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn lease_missing_or_rebound_account_is_rejected_before_authorization() {
    let server = Server::new(vec![]);
    run(async {
        let leases = leases();
        let mut f = Fixture::new(&server, Supplier::default(), policy());
        f.provider = f.provider.with_account_leases(leases.clone());
        let error = f.admit(Route::Responses, deadline()).await.err().unwrap();
        assert_eq!(error.kind, FailureKind::InvalidScope);
        let f = attach(f, &leases).await;
        let mut change = leases.begin_change(f.scope.account()).unwrap().await.unwrap();
        change.fence_user("fixture-rebound-user").unwrap().await.unwrap();
        change.stage_binding(f.scope.account(), Some("fixture-rebound-user")).unwrap();
        change.finish(ChangeResult::Completed).unwrap();
        let error = f.admit(Route::Responses, deadline()).await.err().unwrap();
        assert_eq!(error.kind, FailureKind::InvalidScope);
        assert_eq!(leases.stats().unwrap().operations, 0);
        assert_eq!(leases.stats().unwrap().waiters, 0);
        assert_eq!(f.capacity.available_permits(), 1);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn lease_closed_and_poisoned_accounts_remain_ineligible() {
    let server = Server::new(vec![]);
    run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        leases.begin_change(f.scope.account()).unwrap().await.unwrap()
            .finish(ChangeResult::RetryableFailure).unwrap();
        let error = f.admit(Route::Responses, deadline()).await.err().unwrap();
        assert_eq!(error.kind, FailureKind::Ineligible);
        let mut change = leases.begin_change(f.scope.account()).unwrap().await.unwrap();
        change.fence_user(&f.scope.active_user().0).unwrap().await.unwrap();
        change.stage_binding(f.scope.account(), Some(&f.scope.active_user().0)).unwrap();
        change.finish(ChangeResult::Completed).unwrap();
        let owned = leases.operation(f.scope.account(), &f.scope.active_user().0)
            .unwrap().await.unwrap();
        drop(owned);
        let error = f.admit(Route::Responses, deadline()).await.err().unwrap();
        assert_eq!(error.kind, FailureKind::Ineligible);
        assert_eq!(leases.stats().unwrap().poisoned_accounts, 1);
        assert_eq!(leases.stats().unwrap().waiters, 0);
        assert_eq!(f.capacity.available_permits(), 1);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[derive(Clone, Copy)]
enum LeasePath { Collected, Raw, Composed }

fn discard_value<T: Send + 'static>(run: InvocationRun<T>) -> InvocationRun<()> {
    let InvocationRun { result, cleanup, cancellation, completed_attempts } = run;
    InvocationRun { result: result.map(|value| value.map(|_| ())).boxed(),
        cleanup, cancellation, completed_attempts }
}

fn launch_path(call: Box<dyn OwnedInvocation>, path: LeasePath) -> InvocationRun<()> {
    match path {
        LeasePath::Collected => discard_value(call.responses(responses_payload())),
        LeasePath::Raw => discard_value(call.raw_stream(stream_payload(), limits()).response),
        LeasePath::Composed => discard_value(call.composed_stream_for_client(responses_payload(),
            options(ClientDialect::OpenAiResponses), "fixture-client-alias".to_owned()).response),
    }
}

fn path_route(path: LeasePath) -> Route {
    match path { LeasePath::Raw => Route::Raw(ClientDialect::OpenAiResponses), _ => Route::Responses }
}

#[derive(Clone, Copy)]
enum Interrupt { Cancel, DropResult, Deadline }

fn held_authorization_case(path: LeasePath, interrupt: Interrupt) {
    let server = Server::new(vec![]);
    run(async {
        let leases = leases();
        let mut f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let source = install_held_source(&mut f, false);
        let until = if matches!(interrupt, Interrupt::Deadline) {
            Instant::now() + Duration::from_millis(500)
        } else { deadline() };
        let started = launch_path(f.admit(path_route(path), until).await.unwrap(), path);
        tokio::time::timeout(STEP, source.entered.acquire()).await.unwrap().unwrap().forget();
        if matches!(interrupt, Interrupt::DropResult) { drop(started.result); }
        else {
            if matches!(interrupt, Interrupt::Cancel) { started.cancellation.cancel(); }
            let error = tokio::time::timeout(STEP, started.result).await.unwrap().err().unwrap();
            assert_eq!(error.kind, if matches!(interrupt, Interrupt::Deadline) {
                FailureKind::Deadline
            } else { FailureKind::Cancelled });
        }
        assert!(tokio::time::timeout(PAUSE, started.cleanup.clone()).await.is_err());
        assert_eq!(f.capacity.available_permits(), 0);
        assert_eq!(leases.stats().unwrap().operations, 1);
        assert_busy(&leases, &f);
        source.release.add_permits(1);
        cleanup(started.cleanup).await;
        assert_reusable(&leases, &f); f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn lease_nonstream_authorization_survives_cancel_drop_and_deadline() {
    for how in [Interrupt::Cancel, Interrupt::DropResult, Interrupt::Deadline] {
        held_authorization_case(LeasePath::Collected, how);
    }
}

#[test]
fn lease_raw_authorization_survives_cancel_drop_and_deadline() {
    for how in [Interrupt::Cancel, Interrupt::DropResult, Interrupt::Deadline] {
        held_authorization_case(LeasePath::Raw, how);
    }
}

#[test]
fn lease_composed_authorization_survives_cancel_drop_and_deadline() {
    for how in [Interrupt::Cancel, Interrupt::DropResult, Interrupt::Deadline] {
        held_authorization_case(LeasePath::Composed, how);
    }
}

#[test]
fn lease_nonstream_cancel_waits_for_actual_body_cleanup() {
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", responses_payload(), 0,
        Reply::json(json!({"fixture":"real body cleanup"})))]);
    run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let (release, tail) = oneshot::channel();
        let started = leased_hook_call(&f, deadline()).launch(move |parts, registry| async move {
            let pending = pending_with_hooks(&parts, responses_payload(),
                TestHooks { tail_gate: Some(tail), ..TestHooks::default() });
            registry.started(pending.cleanup.clone(), pending.cancellation.clone());
            let response = pending.response.await.map_err(|_| DispatchError::new(FailureKind::Transport))?;
            let mut body = response.proxy.stream;
            while let Some(chunk) = body.next().await {
                chunk.map_err(|_| DispatchError::new(FailureKind::Body))?;
            }
            Ok(())
        });
        wait_count(&server.shared.replies, 1).await;
        started.cancellation.cancel();
        assert_eq!(started.result.await.err().unwrap().kind, FailureKind::Cancelled);
        assert!(tokio::time::timeout(PAUSE, started.cleanup.clone()).await.is_err());
        assert_eq!(f.capacity.available_permits(), 0);
        assert_busy(&leases, &f);
        release.send(()).unwrap(); cleanup(started.cleanup).await;
        assert_reusable(&leases, &f); f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn lease_raw_eof_waits_for_last_returned_byte_owner() {
    let mut reply = Reply::json(Value::Null); reply.body = b"abcde".to_vec();
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", stream_payload(), 0, reply)]);
    run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let started = start(&f, deadline(), limits()).await;
        let mut head = started.response.result.await.unwrap();
        let bytes = head.body.next().await.unwrap().unwrap();
        let copied = bytes.clone(); let sliced = bytes.slice(..1);
        let mut received = bytes.to_vec(); received.extend(drain(head.body).await);
        assert_eq!(received, b"abcde");
        assert_eq!(finished(started.finished).await.result, Ok(()));
        drop(bytes); drop(copied);
        assert!(tokio::time::timeout(PAUSE, started.response.cleanup.clone()).await.is_err());
        assert_eq!(sliced.as_ref(), b"a");
        assert_busy(&leases, &f);
        assert_eq!(f.capacity.available_permits(), 0);
        drop(sliced); cleanup(started.response.cleanup).await;
        assert_reusable(&leases, &f); f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn lease_composed_continuation_has_one_owner_through_last_frame() {
    let (first, next) = responses_continuation();
    let mut last = response_body();
    last["id"] = json!("resp_last");
    last["output"] = json!([last["output"][1].clone()]);
    let server = Server::new(vec![
        expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(first.clone())),
        expected("/ai/v3/responses/", next.clone(), 1, Reply::json(last.clone()))]);
    run(async {
        let leases = leases();
        let (mut supplier, release) = Supplier::held(); supplier.gate_index = 1;
        let mut configured = policy(); configured.continuation = Some(CALLER_CONTINUE.into());
        let f = attach(Fixture::new(&server, supplier, configured), &leases).await;
        let until = deadline();
        let started = f.admit(Route::Responses, until).await.unwrap().composed_stream_for_client(
            responses_payload(), options(ClientDialect::OpenAiResponses), "fixture-client-alias".into());
        let head = started.response.result.await.unwrap();
        let mut body = head.body;
        let mut competitor = Box::pin(leases.operation(f.scope.account(),
            &f.scope.active_user().0).unwrap());
        let mut bytes = Vec::new();
        while f.supplier.calls.load(Ordering::Acquire) < 2 {
            assert!(competitor.as_mut().now_or_never().is_none());
            tokio::select! {
                _ = wait_count(&f.supplier.calls, 2) => {},
                chunk = body.next() => bytes.extend_from_slice(&chunk.unwrap().unwrap()),
            }
        }
        assert!(competitor.as_mut().now_or_never().is_none());
        assert_eq!(leases.stats().unwrap().operations, 1);
        assert_eq!(leases.stats().unwrap().waiters, 1);
        release.send(()).unwrap();
        let retained = body.next().await.unwrap().unwrap();
        bytes.extend_from_slice(&retained);
        while let Some(chunk) = body.next().await {
            assert!(competitor.as_mut().now_or_never().is_none());
            bytes.extend_from_slice(&chunk.unwrap());
        }
        let done = summary(started.finished).await;
        assert_eq!(done.result, Ok(())); assert_eq!(done.attempts.len(), 2);
        assert!(tokio::time::timeout(PAUSE, started.response.cleanup.clone()).await.is_err());
        assert!(competitor.as_mut().now_or_never().is_none());
        assert_eq!(leases.stats().unwrap().operations, 1);
        assert_eq!(f.capacity.available_permits(), 0);
        let emitted = events(&bytes);
        let output = completed_output(&emitted);
        assert_eq!(output, &json!([first["output"][0], first["output"][1], last["output"][0]]));
        for event in emitted.iter().filter(|event| event["response"].is_object()) {
            assert_eq!(event["response"]["model"], "fixture-client-alias");
        }
        seen(&f.supplier, &f.scope, until, "OpenRouterResponses", &[expected_responses(), next]);
        drop(retained); cleanup(started.response.cleanup).await;
        tokio::time::timeout(STEP, competitor).await.unwrap().unwrap().complete().unwrap();
        assert_reusable(&leases, &f); f.finish().await;
    });
    checked(&server.finish(), 2, 2);
}

#[test]
fn lease_composed_refusal_waits_for_last_clone_and_preserves_reuse() {
    let wire = vec![0, 255, b'x', b'y', b'z'];
    let mut reply = Reply::json(Value::Null); reply.status = 403; reply.body = wire.clone();
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, reply)]);
    run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let started = compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), deadline()).await;
        let error = started.response.result.await.err().unwrap();
        assert_eq!(error.kind, FailureKind::Upstream);
        let upstream = error.upstream.unwrap();
        assert_eq!(upstream.metadata.status, 403);
        assert_eq!(upstream.body.as_ref(), wire.as_slice());
        let cloned = upstream.body.clone(); let sliced = upstream.body.slice(1..3);
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::Upstream));
        drop(upstream); drop(cloned);
        assert!(tokio::time::timeout(PAUSE, started.response.cleanup.clone()).await.is_err());
        assert_eq!(sliced.as_ref(), &wire[1..3]);
        assert_eq!(f.capacity.available_permits(), 0);
        assert_busy(&leases, &f);
        drop(sliced); cleanup(started.response.cleanup).await;
        assert_reusable(&leases, &f);
        assert_eq!(leases.stats().unwrap().poisoned_accounts, 0);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

fn failed_authorization_case(path: LeasePath) {
    let server = Server::new(vec![]);
    let (capacity, supervisor, leases) = run(async {
        let leases = leases();
        let mut f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let other = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let source = install_held_source(&mut f, true);
        let started = launch_path(f.admit(path_route(path), deadline()).await.unwrap(), path);
        tokio::time::timeout(STEP, source.entered.acquire()).await.unwrap().unwrap().forget();
        started.cancellation.cancel();
        assert_eq!(started.result.await.err().unwrap().kind, FailureKind::Cancelled);
        assert!(tokio::time::timeout(PAUSE, started.cleanup.clone()).await.is_err());
        assert_busy(&leases, &f);
        source.release.add_permits(1);
        assert_eq!(tokio::time::timeout(STEP, started.cleanup).await.unwrap(), Err(FailureKind::Cleanup));
        assert_eq!(leases.stats().unwrap().operations, 0);
        assert_eq!(leases.stats().unwrap().poisoned_accounts, 1);
        assert_eq!(leases.operation(f.scope.account(), &f.scope.active_user().0).err(),
            Some(LeaseError::Poisoned));
        drop(other.admit(Route::Responses, deadline()).await.unwrap());
        assert_reusable(&leases, &other); other.finish().await;
        let capacity = f.capacity.clone(); let supervisor = f.client.supervisor_for_test();
        drop(f);
        assert_eq!(capacity.available_permits(), 0); assert!(!supervisor.is_finished());
        (capacity, supervisor, leases)
    });
    assert_eq!(capacity.available_permits(), 1); assert!(supervisor.is_finished());
    assert_eq!(leases.stats().unwrap().poisoned_accounts, 1);
    checked(&server.finish(), 0, 0);
}

#[test]
fn lease_nonstream_failed_authorization_cleanup_poisons_only_its_account() {
    failed_authorization_case(LeasePath::Collected);
}
#[test]
fn lease_raw_failed_authorization_cleanup_poisons_only_its_account() {
    failed_authorization_case(LeasePath::Raw);
}
#[test]
fn lease_composed_failed_authorization_cleanup_poisons_only_its_account() {
    failed_authorization_case(LeasePath::Composed);
}

async fn failed_native_cleanup(f: &Fixture, path: LeasePath) -> port::Cleanup {
    let mut call = leased_hook_call(f, deadline());
    let hooks = TestHooks { lose_native_closed: true, ..TestHooks::default() };
    match path {
        LeasePath::Collected => {
            let started = call.launch(move |parts, registry| async move {
                let pending = pending_with_hooks(&parts, responses_payload(), hooks);
                registry.started(pending.cleanup.clone(), pending.cancellation.clone());
                let response = pending.response.await
                    .map_err(|_| DispatchError::new(FailureKind::Transport))?;
                let mut body = response.proxy.stream;
                while let Some(chunk) = body.next().await {
                    chunk.map_err(|_| DispatchError::new(FailureKind::Body))?;
                }
                Ok(())
            });
            assert!(started.result.await.is_err());
            started.cleanup
        }
        LeasePath::Raw => {
            let started = super::raw_stream::launch_for_test(call, stream_payload(), limits(),
                Box::new(move |parts, payload| Ok(pending_with_hooks(parts, payload, hooks))));
            let head = started.response.result.await.unwrap();
            let (_, error) = composed_outcome(head.body).await;
            assert_eq!(error, Some(FailureKind::Cleanup));
            assert_eq!(finished(started.finished).await.result, Err(FailureKind::Cleanup));
            started.response.cleanup
        }
        LeasePath::Composed => {
            call.request.route = Route::Responses;
            let started = super::composed_stream::launch_for_test(call, responses_payload(),
                options(ClientDialect::OpenAiResponses),
                Box::new(move |parts, payload| Ok(pending_with_hooks(parts, payload, hooks))));
            let head = started.response.result.await.unwrap();
            let (bytes, error) = composed_outcome(head.body).await;
            no_success(&events(&bytes));
            if let Some(kind) = error { assert_eq!(kind, FailureKind::Cleanup); }
            assert_eq!(summary(started.finished).await.result, Err(FailureKind::Cleanup));
            started.response.cleanup
        }
    }
}

fn failed_native_case(path: LeasePath) {
    let payload = match path {
        LeasePath::Collected => responses_payload(), LeasePath::Raw => stream_payload(),
        LeasePath::Composed => expected_responses(),
    };
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", payload, 0,
        Reply::json(response_body()))]);
    let (capacity, supervisor, leases) = run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let other = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let cleanup = failed_native_cleanup(&f, path).await;
        assert_eq!(tokio::time::timeout(STEP, cleanup).await.unwrap(), Err(FailureKind::Cleanup));
        assert_eq!(leases.stats().unwrap().operations, 0);
        assert_eq!(leases.stats().unwrap().poisoned_accounts, 1);
        assert_eq!(leases.operation(f.scope.account(), &f.scope.active_user().0).err(),
            Some(LeaseError::Poisoned));
        drop(other.admit(Route::Responses, deadline()).await.unwrap());
        assert_reusable(&leases, &other); other.finish().await;
        let capacity = f.capacity.clone(); let supervisor = f.client.supervisor_for_test();
        drop(f);
        assert_eq!(capacity.available_permits(), 0); assert!(!supervisor.is_finished());
        (capacity, supervisor, leases)
    });
    assert_eq!(capacity.available_permits(), 1); assert!(supervisor.is_finished());
    assert_eq!(leases.stats().unwrap().poisoned_accounts, 1);
    checked(&server.finish(), 1, 1);
}

#[test]
fn lease_nonstream_failed_native_cleanup_poisons_only_its_account() {
    failed_native_case(LeasePath::Collected);
}
#[test]
fn lease_raw_failed_native_cleanup_poisons_only_its_account() {
    failed_native_case(LeasePath::Raw);
}
#[test]
fn lease_composed_failed_native_cleanup_poisons_only_its_account() {
    failed_native_case(LeasePath::Composed);
}

fn failed_scope_while_credit_is_held(raw: bool) {
    let mut reply = Reply::json(response_body());
    if raw { reply.body = b"abcde".to_vec(); }
    let server = Server::new(vec![expected(
        if raw { "/ai/v3/openai/responses/" } else { "/ai/v3/responses/" },
        if raw { stream_payload() } else { expected_responses() }, 0, reply)]);
    let (capacity, leases) = run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let other = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let (retained, cleanup) = if raw {
            let started = start(&f, deadline(), limits()).await;
            let mut head = started.response.result.await.unwrap();
            let retained = head.body.next().await.unwrap().unwrap();
            let mut received = retained.to_vec(); received.extend(drain(head.body).await);
            assert_eq!(received, b"abcde");
            assert_eq!(finished(started.finished).await.result, Ok(()));
            (retained, started.response.cleanup)
        } else {
            let started = compose(&f, Route::Responses, responses_payload(),
                options(ClientDialect::OpenAiResponses), deadline()).await;
            let mut head = started.response.result.await.unwrap();
            let retained = head.body.next().await.unwrap().unwrap();
            let _remaining = composed_bytes(head.body).await;
            assert_eq!(summary(started.finished).await.result, Ok(()));
            (retained, started.response.cleanup)
        };
        assert!(tokio::time::timeout(PAUSE, cleanup.clone()).await.is_err());
        assert_busy(&leases, &f);
        // The actual native/child cleanup succeeded. Output ownership alone
        // still prevents the outer guardian from releasing the shared lease.
        let supervisor = f.client.supervisor_for_test(); supervisor.abort();
        assert_eq!(tokio::time::timeout(STEP, f.scope.closed()).await.unwrap(),
            Err(crate::experimental_native::native_account::CloseError::SupervisorLost));
        drop(retained);
        assert_eq!(tokio::time::timeout(STEP, cleanup).await.unwrap(), Err(FailureKind::Cleanup));
        assert_eq!(leases.stats().unwrap().operations, 0);
        assert_eq!(leases.operation(f.scope.account(), &f.scope.active_user().0).err(),
            Some(LeaseError::Poisoned));
        drop(other.admit(Route::Responses, deadline()).await.unwrap());
        assert_reusable(&leases, &other); other.finish().await;
        let capacity = f.capacity.clone(); drop(f);
        assert_eq!(capacity.available_permits(), 0);
        (capacity, leases)
    });
    assert_eq!(capacity.available_permits(), 1);
    assert_eq!(leases.stats().unwrap().poisoned_accounts, 1);
    checked(&server.finish(), 1, 1);
}

#[test]
fn lease_raw_late_scope_failure_poisons_after_successful_native_cleanup() {
    failed_scope_while_credit_is_held(true);
}
#[test]
fn lease_composed_late_scope_failure_poisons_after_successful_child_cleanup() {
    failed_scope_while_credit_is_held(false);
}

// Fault hooks need the concrete call. Use the same private admission adapter
// as prepare; the other cases enter through the public provider.
fn leased_hook_call(fixture: &Fixture, until: Instant) -> Box<super::NativeCall> {
    let mut call = call_for_hook(fixture, until);
    let leases = fixture.provider.account_leases.as_ref().expect("attached coordinator");
    call.account_lease = super::shared_lease::AccountLease::admit(Some(leases),
        fixture.scope.account(), &fixture.scope.active_user().0, None).unwrap();
    call
}
