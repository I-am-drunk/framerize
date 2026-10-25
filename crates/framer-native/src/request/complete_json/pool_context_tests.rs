use super::*;
use crate::request::types::PoolAdmission;
use framer_core::operation_gate::OperationGate;
use framer_core::operation_gate::OperationKind;
use framer_core::resident_pool::Acquire;
use framer_core::resident_pool::ResidentPool;

fn admission(account: AccountId, pool: &ResidentPool, gate: &OperationGate)
    -> PoolAdmission {
    let Acquire::Lease(lease) = pool.try_acquire(account, None).unwrap()
        else { panic!("fixture pool has capacity"); };
    PoolAdmission::new(lease, Some(gate.admit(OperationKind::Inference).unwrap()))
}

fn released(f: &Fixture, pool: &ResidentPool, gate: &OperationGate) {
    let state = pool.snapshot().unwrap();
    assert_eq!(state.residents[0].users, 0);
    assert!(!state.residents[0].uncertain);
    assert_eq!(gate.stats().unwrap().active, 0);
    assert!(!gate.stats().unwrap().unconfirmed);
    assert_eq!(f.capacity.available_permits(), 1);
}

async fn start(f: &Fixture, source: Arc<dyn CredentialSource>, route: FreeRoute,
    payload: Value, admission: PoolAdmission) -> Result<super::super::super::ProbeRun, DispatchError> {
    Plan::new(f.scope.clone(), specification(f, route, payload)).unwrap()
        .launch_with_source_in_pool(f.client.clone(),
            source, f.capacity.clone().try_acquire_owned().unwrap(), admission).await
}

fn completed(route: FreeRoute) {
    let (path, payload, response) = match route {
        FreeRoute::Responses => ("/ai/v3/responses/", expected_responses(), response_body()),
        FreeRoute::Messages => ("/ai/v3/messages/", expected_messages(), message_body()),
    };
    let server = Server::new(vec![expected(path, payload, 0, Reply::json(response))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let pool = ResidentPool::new(2).unwrap();
        let gate = OperationGate::new(8).unwrap();
        let source = Arc::new(MaterialSource(AtomicUsize::new(0)));
        let payload = match route {
            FreeRoute::Responses => responses_payload(),
            FreeRoute::Messages => messages_payload(),
        };
        let probe = start(&f, source.clone(), route, payload,
            admission(f.scope.account(), &pool, &gate)).await.unwrap();
        let report = probe_result(probe).await;
        assert!(report.accepted);
        assert_eq!(report.path, path);
        assert_eq!(report.authorization_calls, 1);
        assert_eq!(report.transport_starts, 1);
        assert_eq!(source.0.load(Ordering::SeqCst), 1);
        released(&f, &pool, &gate);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn supplied_source_responses_reuses_pool_admission() { completed(FreeRoute::Responses); }

#[test]
fn supplied_source_messages_reuses_pool_admission() { completed(FreeRoute::Messages); }

#[test]
fn another_accounts_pool_admission_is_rejected_before_issuance() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let pool = ResidentPool::new(2).unwrap();
        let gate = OperationGate::new(8).unwrap();
        let source = Arc::new(MaterialSource(AtomicUsize::new(0)));
        let error = start(&f, source.clone(), FreeRoute::Responses, responses_payload(),
            admission(AccountId(uuid::Uuid::new_v4()), &pool, &gate))
            .await.err().unwrap();
        assert_eq!(error.kind, FailureKind::InvalidScope);
        assert_eq!(source.0.load(Ordering::SeqCst), 0);
        released(&f, &pool, &gate);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

struct HeldMaterial {
    entered: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
    inner: MaterialSource,
}
impl CredentialSource for HeldMaterial {
    fn issue<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        control: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async move {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            self.inner.issue(scope, session, control).await
        }.boxed()
    }
}

fn abandoned(cancel: bool) {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let pool = ResidentPool::new(2).unwrap();
        let gate = OperationGate::new(8).unwrap();
        let source = Arc::new(HeldMaterial {
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
            inner: MaterialSource(AtomicUsize::new(0)),
        });
        let probe = start(&f, source.clone(), FreeRoute::Responses, responses_payload(),
            admission(f.scope.account(), &pool, &gate)).await.unwrap();
        tokio::time::timeout(STEP, source.entered.acquire()).await.unwrap().unwrap().forget();
        let mut result = Some(probe.result);
        if cancel { probe.cancellation.cancel(); }
        else { drop(result.take()); }
        assert!(tokio::time::timeout(Duration::from_millis(20), probe.cleanup.clone()).await.is_err());
        assert_eq!(pool.snapshot().unwrap().residents[0].users, 1);
        assert_eq!(gate.stats().unwrap().active, 1);
        assert_eq!(f.capacity.available_permits(), 0);
        source.release.add_permits(1);
        if let Some(result) = result {
            let report = tokio::time::timeout(STEP, result).await.unwrap();
            assert_eq!(report.failure, Some(FailureKind::Cancelled));
            assert!(!report.accepted);
            assert!(report.cleanup_confirmed);
        }
        assert_eq!(tokio::time::timeout(STEP, probe.cleanup).await.unwrap(), Ok(()));
        released(&f, &pool, &gate);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn cancelled_pool_plan_retains_admission_until_source_cleanup() { abandoned(true); }

#[test]
fn dropped_pool_plan_retains_admission_until_source_cleanup() { abandoned(false); }
