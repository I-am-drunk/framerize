use super::*;
use framer_core::{ids::{AccountId, FramerUserId, ProjectId, TeamId}, port::Session};
use framer_api::{experimental_native::{native_account::{PreparationControl,
    PreparationResult, PreparationFailure}, authorization::CredentialMaterial},
    native_dispatch::complete_json::{FreeRoute, ContentType}};
use std::sync::{Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}};
use tokio::sync::Semaphore;

#[path = "browser_comparison_tests.rs"]
mod comparison;

struct Store { loads: AtomicUsize, user: FramerUserId }
impl SessionStore for Store {
    fn load<'a, 'b, 'f>(&'a self, _: &'b FramerUserId)
        -> FutureResult<'f, Result<Option<Session>, String>> where 'a: 'f, 'b: 'f, Self: 'f {
        Box::pin(async move {
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(Some(Session { active_user: self.user.clone(),
                cookie_header: "session=fixture-secret".into() }))
        })
    }
    fn save<'a, 'b, 'f>(&'a self, _: &'b Session)
        -> FutureResult<'f, Result<(), String>> where 'a: 'f, 'b: 'f, Self: 'f {
        Box::pin(async { panic!("control must never save sessions") })
    }
    fn delete<'a, 'b, 'f>(&'a self, _: &'b FramerUserId)
        -> FutureResult<'f, Result<(), String>> where 'a: 'f, 'b: 'f, Self: 'f {
        Box::pin(async { panic!("control must never delete sessions") })
    }
}
struct Window {
    selected: Mutex<Option<Result<Selection, Failure>>>,
    reports: Mutex<Vec<LocalReport>>, recorded: Semaphore, record_failure: AtomicBool,
}
impl ExclusiveWindow for Window {
    fn admit(&self) -> FutureResult<'_, Result<Selection, Failure>> {
        Box::pin(async { self.selected.lock().unwrap().take().expect("one admission") })
    }
    fn record(&self, report: &LocalReport) -> FutureResult<'_, Result<(), Failure>> {
        self.reports.lock().unwrap().push(report.clone());
        self.recorded.add_permits(1);
        Box::pin(async { if self.record_failure.load(Ordering::SeqCst) {
            Err(Failure::CooldownPublication)
        } else { Ok(()) } })
    }
}
struct RejectedSource { calls: AtomicUsize, entered: Semaphore, release: Semaphore,
    uncertain: AtomicBool }
impl CredentialSource for RejectedSource {
    fn issue<'a>(&'a self, _: &'a AccountScope, _: &'a Session,
        _: &'a PreparationControl) -> FutureResult<'a, PreparationResult<CredentialMaterial>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            if self.uncertain.load(Ordering::SeqCst) { PreparationResult::unconfirmed() }
            else { PreparationResult::finished(Err(PreparationFailure::Rejected)) }
        })
    }
}
struct Builder {
    builds: AtomicUsize, source: Arc<RejectedSource>, expected_proxy: Option<String>,
}
impl SourceBuilder for Builder {
    fn build(&self, scope: AccountScope, proxy: Option<String>)
        -> Result<Arc<dyn CredentialSource>, Failure> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        assert_eq!(proxy, self.expected_proxy);
        assert!(scope.matches_egress(&proxy.as_deref().map(NativeEgress::Proxy)
            .unwrap_or(NativeEgress::Direct)));
        Ok(self.source.clone())
    }
}
fn context() -> AttemptContext {
    AttemptContext { feature_category: "fixture".into(), feature_name: "fixture".into(),
        step: "fixture-step".into(), turn_id: "fixture-turn".into(), user_prompt: "fixture".into() }
}
fn identity() -> Identity {
    Identity { account: AccountId::new(), user: FramerUserId("fixture-user".into()),
        team: TeamId("fixture-team".into()), project: ProjectId("fixture-project".into()),
        chat_session: "fixture-conversation".into() }
}
fn spec(identity: Identity) -> Specification {
    Specification { identity, route: FreeRoute::Responses, context: context(), token_limit: 32,
        payload: serde_json::json!({"model":"fixture-model","stream":false,"input":"fixture"}),
        deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(30) }
}
fn fixture(id: &Identity, proxy: Option<String>) -> (Arc<Store>, Arc<Builder>, Arc<Window>) {
    let store = Arc::new(Store { loads: AtomicUsize::new(0), user: id.user.clone() });
    let source = Arc::new(RejectedSource { calls: AtomicUsize::new(0),
        entered: Semaphore::new(0), release: Semaphore::new(1), uncertain: AtomicBool::new(false) });
    let builder = Arc::new(Builder { builds: AtomicUsize::new(0), source,
        expected_proxy: proxy.clone() });
    let window = Arc::new(Window { selected: Mutex::new(Some(Ok(Selection {
        identity: id.clone(), context: context(), proxy }))),
        reports: Mutex::new(Vec::new()), recorded: Semaphore::new(0),
        record_failure: AtomicBool::new(false) });
    (store, builder, window)
}
fn deps(store: Arc<Store>, source: Arc<Builder>, authority: Option<Arc<Window>>) -> Dependencies {
    Dependencies { store, source,
        authority: authority.map(|v| v as Arc<dyn ExclusiveWindow>),
        permit: Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap() }
}
async fn bounded(f: impl Future<Output = LocalReport>) -> LocalReport {
    tokio::time::timeout(std::time::Duration::from_secs(3), f).await.expect("bounded caller")
}
#[tokio::test]
async fn missing_authority_rejects_before_private_load_or_source() {
    let id = identity();
    let (store, builder, _) = fixture(&id, None);
    let report = bounded(launch(spec(id), deps(store.clone(), builder.clone(), None)).unwrap()).await;
    assert_eq!(report.failure, Some(Failure::MissingAuthority));
    assert_eq!(store.loads.load(Ordering::SeqCst), 0);
    assert_eq!(builder.builds.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn cooling_and_wave_pause_reject_before_private_load_or_source() {
    for failure in [Failure::Cooling, Failure::WavePaused] {
        let id = identity(); let (store, builder, window) = fixture(&id, None);
        *window.selected.lock().unwrap() = Some(Err(failure));
        let report = bounded(launch(spec(id), deps(store.clone(), builder.clone(), Some(window))).unwrap()).await;
        assert_eq!(report.failure, Some(failure));
        assert_eq!(store.loads.load(Ordering::SeqCst), 0);
        assert_eq!(builder.builds.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn identity_and_context_mismatches_reject_before_private_load_or_source() {
    for field in 0..6 {
        let id = identity(); let (store, builder, window) = fixture(&id, None);
        let mut request = spec(id);
        match field {
            0 => request.identity.account = AccountId::new(),
            1 => request.identity.user.0.push('x'),
            2 => request.identity.project.0.push('x'),
            3 => request.identity.team.0.push('x'),
            4 => request.identity.chat_session.push('x'),
            _ => request.context.turn_id.push('x'),
        }
        let report = bounded(launch(request, deps(store.clone(), builder.clone(), Some(window))).unwrap()).await;
        assert_eq!(report.failure, Some(Failure::Context));
        assert_eq!(store.loads.load(Ordering::SeqCst), 0);
        assert_eq!(builder.builds.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn stored_user_mismatch_never_creates_authorization_source() {
    let id = identity(); let (_, builder, window) = fixture(&id, None);
    let store = Arc::new(Store { loads: AtomicUsize::new(0), user: FramerUserId("wrong".into()) });
    let report = bounded(launch(spec(id), deps(store.clone(), builder.clone(), Some(window))).unwrap()).await;
    assert_eq!(report.failure, Some(Failure::Session));
    assert_eq!(store.loads.load(Ordering::SeqCst), 1);
    assert_eq!(builder.builds.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn explicit_egress_reaches_native_scope_and_source_with_one_authorization() {
    for proxy in [None, Some("http://127.0.0.1:9".into())] {
        let id = identity(); let (store, builder, window) = fixture(&id, proxy);
        let report = bounded(launch(spec(id), deps(store, builder.clone(), Some(window.clone()))).unwrap()).await;
        assert_eq!(builder.builds.load(Ordering::SeqCst), 1);
        assert_eq!(builder.source.calls.load(Ordering::SeqCst), 1);
        assert_eq!(report.transport_starts, 0);
        assert_eq!(report.dispatch_failure, Some(FailureKind::Authorization));
        assert!(report.local_cleanup);
        assert_eq!(window.reports.lock().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn dropped_result_waiter_retains_cleanup_and_policy_owner() {
    let id = identity(); let (store, builder, window) = fixture(&id, None);
    builder.source.release.try_acquire().unwrap().forget();
    let waiter = launch(spec(id), deps(store, builder.clone(), Some(window.clone()))).unwrap();
    let entered = tokio::time::timeout(std::time::Duration::from_secs(3),
        builder.source.entered.acquire()).await.unwrap().unwrap();
    entered.forget();
    drop(waiter);
    assert!(window.reports.lock().unwrap().is_empty());
    builder.source.release.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(3), window.recorded.acquire())
        .await.unwrap().unwrap().forget();
    let reports = window.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert!(reports[0].local_cleanup);
    assert_eq!(builder.source.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn bootstrap_commits_binding_before_normal_operation_admission() {
    let id = identity();
    let leases = AccountLeaseCoordinator::new(LeaseLimits { accounts: 1, users: 1,
        waiters: 2, user_id_bytes: 256, total_user_bytes: 256 }).unwrap();
    assert!(leases.operation(id.account, &id.user.0).is_err());
    bootstrap(&leases, &id).await.unwrap();
    let operation = leases.operation(id.account, &id.user.0).unwrap().await.unwrap();
    operation.complete().unwrap();
    assert_eq!(leases.stats().unwrap().operations, 0);
}
#[tokio::test]
async fn unconfirmed_authorization_cannot_become_successful_cleanup() {
    let id = identity(); let (store, builder, window) = fixture(&id, None);
    builder.source.uncertain.store(true, Ordering::SeqCst);
    let report = bounded(launch(spec(id), deps(store, builder.clone(), Some(window))).unwrap()).await;
    assert!(!report.local_cleanup);
    assert_eq!(report.failure, Some(Failure::Cleanup));
    // Scope closure can race the rejected authorization result. Preserve
    // whichever bounded class the existing adapter actually observed.
    assert!(matches!(report.dispatch_failure, Some(FailureKind::Authorization
        | FailureKind::ScopeClosed | FailureKind::Cleanup)));
    assert_eq!(builder.source.calls.load(Ordering::SeqCst), 1);
    assert_eq!(report.transport_starts, 0);
}
fn evidence(cleanup: bool, status: u16) -> Evidence {
    Evidence { accepted: cleanup && status == 200, path: "/ai/v3/responses/",
        client_model: "session=fixture-secret".into(), wire_model: "Bearer fixture-secret".into(),
        status: Some(status), content_type: ContentType::Json, usage_header_present: true,
        input_tokens: Some(1), output_tokens: Some(1), output_items: 1,
        attempts: 1, authorization_calls: 1, transport_starts: 1, transport_finishes: 1,
        complete_json: status == 200, cleanup_confirmed: cleanup, refusal: None,
        failure: if !cleanup { Some(FailureKind::Cleanup) }
            else if status != 200 { Some(FailureKind::Upstream) } else { None } }
}
#[test]
fn successful_json_with_failed_cleanup_remains_failure() {
    let report = LocalReport::from_evidence(evidence(false, 200));
    assert!(report.complete_json);
    assert!(!report.local_cleanup);
    assert!(report.failure.is_some());
    assert_eq!(report.dispatch_failure, Some(FailureKind::Cleanup));
}
#[test]
fn refusal_projection_retains_status_and_class_without_response_content() {
    let report = LocalReport::from_evidence(evidence(true, 403));
    assert_eq!(report.status, Some(403));
    assert_eq!(report.dispatch_failure, Some(FailureKind::Upstream));
    assert_eq!(report.attempts, 1);
    assert!(!report.complete_json);
    let output = format!("{report:?}");
    for secret in ["session=", "Bearer", "fixture-secret"] { assert!(!output.contains(secret)); }
}

#[test]
fn refusal_projection_keeps_basis_even_when_cleanup_fails() {
    use framer_api::native_dispatch::complete_json::{Refusal, RefusalBasis, RefusalObservation};
    for basis in [RefusalBasis::FramerSecurityEnvelope, RefusalBasis::ElapsedFallback] {
        let mut e = evidence(false, 403);
        let observed = RefusalObservation { policy: Refusal::Castle, basis };
        e.refusal = Some(observed);
        let report = LocalReport::from_evidence(e);
        assert_eq!(report.refusal, Some(observed));
        assert!(!report.local_cleanup);
        assert!(report.failure.is_some());
        assert_eq!(report.dispatch_failure, Some(FailureKind::Cleanup));
    }
    assert_eq!(LocalReport::failed(Failure::Source).refusal, None);
}
#[tokio::test]
async fn policy_publication_failure_preserves_dispatch_class() {
    let id = identity(); let (store, builder, window) = fixture(&id, None);
    window.record_failure.store(true, Ordering::SeqCst);
    let report = bounded(launch(spec(id), deps(store, builder, Some(window))).unwrap()).await;
    assert_eq!(report.failure, Some(Failure::CooldownPublication));
    assert_eq!(report.dispatch_failure, Some(FailureKind::Authorization));
    assert_eq!(report.status, None);
    assert!(report.local_cleanup);
}
