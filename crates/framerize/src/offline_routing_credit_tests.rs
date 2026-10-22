use super::*;

#[tokio::test]
async fn routing_paid_revalidates_account_after_the_credit_read() {
    let (state, ports, spec, sessions) = fixture();
    let ledger = Ledger::new(&spec, 0, true);
    let call = paid(&state, &ports, &spec, &sessions, ledger.clone()).await;
    let run = call.raw(spec.payload.clone());
    wait(&ledger.entered).await;
    ports.account.lock().unwrap().current_project = Some(ProjectId("changed".into()));
    ledger.release.add_permits(1);
    assert_eq!(run.result.await.err().unwrap().kind, FailureKind::InvalidScope);
    assert_eq!(run.cleanup.await, Err(FailureKind::InvalidScope));
    assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
    assert_eq!(users(&state, spec.identity.account).await, 0);
    assert_eq!(ports.account.lock().unwrap().current_project, Some(ProjectId("changed".into())));
}

#[tokio::test]
async fn routing_paid_cancel_or_drop_keeps_credit_cleanup_owned() {
    for drop_result in [false, true] {
        let (state, ports, spec, sessions) = fixture();
        let ledger = Ledger::new(&spec, 0, true);
        let call = paid(&state, &ports, &spec, &sessions, ledger.clone()).await;
        let run = call.raw(spec.payload.clone());
        wait(&ledger.entered).await;
        let mut result = Some(run.result);
        if drop_result { drop(result.take()); } else { run.cancellation.cancel(); }
        assert!(run.cleanup.clone().now_or_never().is_none());
        assert!(users(&state, spec.identity.account).await > 0);
        let key = format!("{}:{}", spec.identity.team.0, convert::to_openrouter_id("gpt-4.1"));
        assert!(state.gate.acquire(&key).now_or_never().is_none());
        tokio::time::timeout(Duration::from_secs(2), async {
            while !ledger.cancelled.load(Ordering::SeqCst) { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
        ledger.release.add_permits(1);
        if let Some(result) = result {
            assert_eq!(result.await.err().unwrap().kind, FailureKind::Cancelled);
        }
        assert_eq!(run.cleanup.await, Ok(()));
        assert_eq!(users(&state, spec.identity.account).await, 0);
    }
}

#[tokio::test]
async fn routing_paid_cancel_before_start_does_not_read_credit() {
    let (state, ports, spec, sessions) = fixture();
    let ledger = Ledger::new(&spec, 0, false);
    let call = paid(&state, &ports, &spec, &sessions, ledger.clone()).await;
    let run = call.raw(spec.payload);
    run.cancellation.cancel();
    assert_eq!(run.result.await.err().unwrap().kind, FailureKind::Cancelled);
    assert_eq!(run.cleanup.await, Ok(()));
    assert_eq!(ledger.reads.load(Ordering::SeqCst), 0);
    assert!(ports.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn routing_paid_worker_loss_retains_the_account() {
    for reply in [3, 4] {
        let (state, ports, spec, sessions) = fixture();
        let ledger = Ledger::new(&spec, reply, false);
        let call = paid(&state, &ports, &spec, &sessions, ledger).await;
        let run = call.raw(spec.payload.clone());
        let error = tokio::time::timeout(Duration::from_secs(2), run.result)
            .await.unwrap().err().unwrap();
        assert_eq!(error.kind, FailureKind::Cleanup);
        assert_eq!(run.cleanup.await, Err(FailureKind::Cleanup));
        assert!(users(&state, spec.identity.account).await > 0);
        assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
        assert!(native_control::admit(&state, spec.identity, "gpt-4.1", spec.deadline)
            .await.is_err());
    }
}

#[tokio::test]
async fn routing_paid_replaces_claimed_credit_with_the_owned_ledger() {
    for (reply, expected, builds) in [(0, FailureKind::Authorization, 1),
        (1, FailureKind::CreditFloor, 0), (2, FailureKind::CreditFloor, 0),
        (5, FailureKind::Authorization, 0)] {
        let (state, ports, spec, sessions) = fixture();
        let ledger = Ledger::new(&spec, reply, false);
        let call = paid(&state, &ports, &spec, &sessions, ledger.clone()).await;
        let run = call.raw(spec.payload.clone());
        let error = tokio::time::timeout(Duration::from_secs(2), run.result)
            .await.unwrap().err().unwrap();
        assert_eq!(error.kind, expected);
        assert_eq!(run.cleanup.await, Ok(()));
        assert_eq!(ledger.reads.load(Ordering::SeqCst), 1);
        assert_eq!(ports.builds.load(Ordering::SeqCst), builds);
        assert_eq!(users(&state, spec.identity.account).await, 0);
    }
}
use futures::future::BoxFuture;
use crate::offline_routing::credit::Reader;
use framer_api::{native_dispatch::CreditEvidence,
    experimental_native::native_account::AccountScope};

struct Claimed(Metadata);
impl Contexts for Claimed {
    fn contains_account(&self, account: AccountId) -> bool { self.0.contains_account(account) }
    fn resolve(&self, request: &Preparation, payload: &serde_json::Value)
        -> Result<RequestContext, port::DispatchError> {
        let mut context = self.0.resolve(request, payload)?;
        context.paid_credit = Some(CreditEvidence { available: 1_000_000.0,
            valid_until: tokio::time::Instant::now() + Duration::from_secs(60) });
        Ok(context)
    }
}

impl Reader for Ledger {
    fn read<'a>(&'a self, scope: &'a AccountScope, session: &'a Session, team: &'a TeamId,
        proxy: Option<&'a str>, control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<CreditEvidence>> {
        async move {
            assert_eq!(scope.account(), self.expected.account);
            assert_eq!(session.active_user, self.expected.user);
            assert_eq!(*team, self.expected.team);
            assert_eq!(proxy, None);
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.entered.add_permits(1);
            loop {
                if control.is_cancelled() { self.cancelled.store(true, Ordering::SeqCst); }
                tokio::select! {
                    value = self.release.acquire() => { value.unwrap().forget(); break; }
                    _ = tokio::time::sleep(Duration::from_millis(1)) => {}
                }
            }
            self.answer()
        }.boxed()
    }
}

struct Ledger {
    expected: port::Identity,
    entered: Semaphore,
    release: Semaphore,
    cancelled: AtomicBool,
    reads: AtomicUsize,
    reply: u8,
}

async fn paid(state: &AppState, ports: &Arc<Ports>, spec: &Specification,
    sessions: &Sessions, ledger: Arc<Ledger>) -> Box<dyn OwnedInvocation> {
    paid_route(state, ports, spec, sessions, ledger, convert::ClientDialect::OpenAiResponses).await
}

async fn paid_route(state: &AppState, ports: &Arc<Ports>, spec: &Specification,
    sessions: &Sessions, ledger: Arc<Ledger>, dialect: convert::ClientDialect)
    -> Box<dyn OwnedInvocation> {
    let metadata = Metadata { identity: spec.identity.clone(), proxy: None, missing: false };
    let mut policy = provider(state, ports, spec, sessions, false).policy();
    policy.allow_paid = true;
    let source_ports = ports.clone();
    let provider = Provider::with_sources(state.clone(), sessions.clone(), Arc::new(Claimed(metadata)),
        policy, None, Arc::new(move |_| Arc::new(Builder(source_ports.clone()))))
        .unwrap().with_credits(ledger);
    let admission = native_control::admit(state, spec.identity.clone(), "gpt-4.1", spec.deadline)
        .await.unwrap();
    admission.completion.complete().unwrap();
    ports.events.lock().unwrap().clear();
    let mut wanted = request(spec);
    wanted.route = Route::Raw(dialect);
    provider.prepare_in_pool(wanted, admission.capacity, admission.pool).await.unwrap()
}

impl Ledger {
    fn new(spec: &Specification, reply: u8, held: bool) -> Arc<Self> {
        Arc::new(Self { expected: spec.identity.clone(), entered: Semaphore::new(0),
            release: Semaphore::new(usize::from(!held)), cancelled: AtomicBool::new(false),
            reads: AtomicUsize::new(0), reply })
    }

    fn answer(&self) -> PreparationResult<CreditEvidence> {
        match self.reply {
            3 => return PreparationResult::unconfirmed(),
            4 => panic!("fixture owned credit worker loss"),
            5 => return PreparationResult::finished(Err(PreparationFailure::Rejected)),
            _ => {}
        }
        let now = tokio::time::Instant::now();
        PreparationResult::finished(Ok(CreditEvidence {
            available: if self.reply == 1 { 0.0 } else { 100.0 },
            valid_until: if self.reply == 2 { now - Duration::from_secs(1) }
                else { now + Duration::from_secs(5) },
        }))
    }
}

#[tokio::test]
async fn routing_paid_all_native_routes_reach_the_owned_ledger() {
    for dialect in [convert::ClientDialect::OpenAiResponses,
        convert::ClientDialect::OpenAiChat, convert::ClientDialect::AnthropicMessages] {
        let (state, ports, spec, sessions) = fixture();
        let ledger = Ledger::new(&spec, 1, false);
        let call = paid_route(&state, &ports, &spec, &sessions, ledger.clone(), dialect).await;
        let run = call.raw(spec.payload);
        let error = tokio::time::timeout(Duration::from_secs(2), run.result)
            .await.unwrap().err().unwrap();
        assert_eq!(error.kind, FailureKind::CreditFloor);
        assert_eq!(run.cleanup.await, Ok(()));
        assert_eq!(ledger.reads.load(Ordering::SeqCst), 1);
        assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
        assert_eq!(users(&state, spec.identity.account).await, 0);
    }
}
