use super::*;
use framer_core::{native_dispatch::PoolAdmission,
    operation_gate::{OperationGate, OperationKind},
    resident_pool::{Acquire, PoolError, ResidentPool}};

fn output_peer(raw: bool) -> Server {
    let mut reply = Reply::json(response_body());
    if raw { reply.body = b"abcde".to_vec(); }
    Server::new(vec![expected(
        if raw { "/ai/v3/openai/responses/" } else { "/ai/v3/responses/" },
        if raw { stream_payload() } else { expected_responses() }, 0, reply)])
}

fn output_ownership(raw: bool) {
    let server = output_peer(raw);
    run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let pool = ResidentPool::new(2).unwrap();
        let gate = OperationGate::new(8).unwrap();
        let (held, done) = held_output(&f, &pool, &gate, raw).await;
        assert!(tokio::time::timeout(PAUSE, done.clone()).await.is_err());
        pending(&f, &pool, &gate);
        assert_busy(&leases, &f);
        drop(held);
        cleanup(done).await;
        released(&f, &pool, &gate);
        assert_reusable(&leases, &f);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn raw_output_ownership_keeps_pool_residency_until_cleanup() { output_ownership(true); }
#[test]
fn composed_output_ownership_keeps_pool_residency_until_cleanup() { output_ownership(false); }

#[test]
fn raw_failed_scope_retains_uncertain_pool_residency() { failed_scope(true); }
#[test]
fn composed_failed_scope_retains_uncertain_pool_residency() { failed_scope(false); }

#[test]
fn cancelled_authorization_keeps_pool_owned_until_the_registered_supplier_finishes() {
    let server = Server::new(vec![]);
    run(async {
        let leases = leases();
        let mut f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let source = install_held_source(&mut f, false);
        let pool = ResidentPool::new(2).unwrap();
        let gate = OperationGate::new(8).unwrap();
        let call = admitted(&f, &pool, &gate, Route::Responses).await;
        let started = call.responses(responses_payload());
        tokio::time::timeout(STEP, source.entered.acquire()).await.unwrap().unwrap().forget();
        started.cancellation.cancel();
        drop(started.result);
        assert!(tokio::time::timeout(PAUSE, started.cleanup.clone()).await.is_err());
        pending(&f, &pool, &gate);
        assert_busy(&leases, &f);
        source.release.add_permits(1);
        cleanup(started.cleanup).await;
        released(&f, &pool, &gate);
        assert_reusable(&leases, &f);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

fn failed_scope(raw: bool) {
    let server = output_peer(raw);
    run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let pool = ResidentPool::new(2).unwrap();
        let gate = OperationGate::new(8).unwrap();
        let (held, done) = held_output(&f, &pool, &gate, raw).await;
        pending(&f, &pool, &gate);
        f.client.supervisor_for_test().abort();
        assert!(tokio::time::timeout(STEP, f.scope.closed()).await.unwrap().is_err());
        drop(held);
        assert_eq!(tokio::time::timeout(STEP, done).await.unwrap(), Err(FailureKind::Cleanup));
        assert_failed_pool(&f, &pool, &gate, &leases);
        drop(f);
    });
    checked(&server.finish(), 1, 1);
}

fn pool_admission(f: &Fixture, pool: &ResidentPool, gate: &OperationGate) -> PoolAdmission {
    let Acquire::Lease(resident) = pool.try_acquire(f.scope.account(), None).unwrap()
        else { panic!("fixture has space"); };
    PoolAdmission::new(resident, Some(gate.admit(OperationKind::Inference).unwrap()))
}

fn pending(f: &Fixture, pool: &ResidentPool, gate: &OperationGate) {
    assert_eq!(pool.snapshot().unwrap().residents[0].users, 1);
    assert_eq!(gate.stats().unwrap().active, 1);
    assert!(matches!(pool.begin_park(f.scope.account()), Err(PoolError::AccountBusy)));
    assert_eq!(f.capacity.available_permits(), 0);
}

fn released(f: &Fixture, pool: &ResidentPool, gate: &OperationGate) {
    let state = pool.snapshot().unwrap();
    assert_eq!(state.occupied, 1);
    assert_eq!(state.residents[0].users, 0);
    assert!(!state.residents[0].uncertain);
    assert_eq!(gate.stats().unwrap().active, 0);
    assert!(!gate.stats().unwrap().unconfirmed);
    assert_eq!(f.capacity.available_permits(), 1);
}

fn assert_failed_pool(f: &Fixture, pool: &ResidentPool, gate: &OperationGate,
    leases: &AccountLeaseCoordinator) {
    let snapshot = pool.snapshot().unwrap();
    assert_eq!(snapshot.residents[0].users, 1);
    assert!(snapshot.residents[0].uncertain);
    assert!(matches!(pool.begin_park(f.scope.account()), Err(PoolError::AccountBusy)));
    assert!(gate.stats().unwrap().unconfirmed);
    assert_eq!(leases.operation(f.scope.account(), &f.scope.active_user().0).err(),
        Some(LeaseError::Poisoned));
}

async fn admitted(f: &Fixture, pool: &ResidentPool, gate: &OperationGate, route: Route)
    -> Box<dyn OwnedInvocation> {
    let permit = f.capacity.clone().try_acquire_owned().unwrap();
    f.provider.prepare_in_pool(f.request(route, deadline()), permit,
        pool_admission(f, pool, gate)).await.unwrap()
}

async fn held_output(f: &Fixture, pool: &ResidentPool, gate: &OperationGate, raw: bool)
    -> (bytes::Bytes, port::Cleanup) {
    let route = if raw { Route::Raw(ClientDialect::OpenAiResponses) } else { Route::Responses };
    let call = admitted(f, pool, gate, route).await;
    if raw {
        let started = call.raw_stream(stream_payload(), limits());
        let mut head = started.response.result.await.unwrap();
        let retained = head.body.next().await.unwrap().unwrap();
        let _rest = drain(head.body).await;
        assert_eq!(finished(started.finished).await.result, Ok(()));
        (retained, started.response.cleanup)
    } else {
        let started = call.composed_stream(responses_payload(), options(ClientDialect::OpenAiResponses));
        let mut head = started.response.result.await.unwrap();
        let retained = head.body.next().await.unwrap().unwrap();
        let _rest = composed_bytes(head.body).await;
        assert_eq!(summary(started.finished).await.result, Ok(()));
        (retained, started.response.cleanup)
    }
}

#[test]
fn dropping_unused_pool_preparation_releases_all_three_owners() {
    let server = Server::new(vec![]);
    run(async {
        let leases = leases();
        let f = attach(Fixture::new(&server, Supplier::default(), policy()), &leases).await;
        let pool = ResidentPool::new(2).unwrap();
        let gate = OperationGate::new(8).unwrap();
        let call = admitted(&f, &pool, &gate, Route::Responses).await;
        pending(&f, &pool, &gate);
        assert_busy(&leases, &f);
        drop(call);
        released(&f, &pool, &gate);
        assert_reusable(&leases, &f);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}
