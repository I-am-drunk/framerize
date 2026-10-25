use super::*;
use super::super::FreeRoute;
use super::super::Plan;
use crate::authorization::BoundAttemptContext;
use crate::authorization::ContextPolicy;
use crate::authorization::CredentialMaterial;
use crate::authorization::CredentialSource;
use crate::authorization::GenerationAuthorization;
use crate::account::client::PreparationControl;
use crate::account::client::PreparationResult;

pub(super) struct MaterialSource(pub(super) AtomicUsize);
impl CredentialSource for MaterialSource {
    fn issue<'a>(&'a self, scope: &'a AccountScope, _: &'a Session,
        _: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        futures::future::ready(PreparationResult::finished(Ok(CredentialMaterial {
            bearer: "fixture-application-bearer".into(),
            castle: FreshCastleToken::from_sdk_output(scope.account(),
                CastleToken("fixture-application-castle".into())),
        }))).boxed()
    }
}

fn bound(spec: &super::super::Specification) -> BoundAttemptContext {
    BoundAttemptContext { project: spec.identity.project.clone(),
        chat_session: spec.identity.chat_session.clone(), step_index: 0,
        context: AttemptContext {
            feature_category: spec.context.feature_category.clone(),
            feature_name: spec.context.feature_name.clone(), step: spec.context.step.clone(),
            turn_id: spec.context.turn_id.clone(), user_prompt: spec.context.user_prompt.clone(),
        } }
}

#[test]
fn exact_generation_supplier_completes_responses_plan_with_pool_lease() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
        Reply::json(response_body()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let report = exercise(&f, FreeRoute::Responses, responses_payload(), true).await;
        assert!(report.accepted);
        assert_eq!(report.path, "/ai/v3/responses/");
        assert_eq!(report.authorization_calls, 1);
        assert_eq!(report.transport_starts, 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn exact_generation_supplier_completes_messages_plan_with_pool_lease() {
    let server = Server::new(vec![expected("/ai/v3/messages/", expected_messages(), 0,
        Reply::json(message_body()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let report = exercise(&f, FreeRoute::Messages, messages_payload(), true).await;
        assert!(report.accepted);
        assert_eq!(report.path, "/ai/v3/messages/");
        assert_eq!(report.authorization_calls, 1);
        assert_eq!(report.transport_starts, 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn derived_context_cannot_satisfy_an_exact_one_shot_plan() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let report = exercise(&f, FreeRoute::Responses, responses_payload(), false).await;
        assert!(!report.accepted);
        assert_eq!(report.authorization_calls, 1);
        assert_eq!(report.transport_starts, 0);
        assert!(report.cleanup_confirmed);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn supplied_source_preserves_exact_context_and_pool_lease() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
        Reply::json(response_body()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let source = Arc::new(MaterialSource(AtomicUsize::new(0)));
        let plan = Plan::new(f.scope.clone(), specification(&f,
            FreeRoute::Responses, responses_payload())).unwrap();
        let permit = f.capacity.clone().try_acquire_owned().unwrap();
        let started = plan.launch_with_source_in_pool(f.client.clone(), source.clone(), permit, f.admission()).await.unwrap();
        assert!(probe_result(started).await.accepted);
        assert_eq!(source.0.load(Ordering::SeqCst), 1);
        assert_eq!(f.gate.stats().unwrap().active, 0);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

async fn exercise(f: &Fixture, route: FreeRoute, payload: Value, exact: bool)
    -> super::super::Evidence {
    let spec = specification(f, route, payload);
    let material = Arc::new(MaterialSource(AtomicUsize::new(0)));
    let supplier = if exact {
        GenerationAuthorization::for_attempt(f.client.clone(), f.scope.clone(),
            material.clone(), bound(&spec)).unwrap()
    } else {
        GenerationAuthorization::new(f.client.clone(), f.scope.clone(), material.clone(),
            ContextPolicy { project: spec.identity.project.clone(),
                feature_category: spec.context.feature_category.clone(),
                feature_name: spec.context.feature_name.clone(), step: spec.context.step.clone(),
                display_prompt_chars: 1024 }).unwrap()
    };
    let plan = Plan::new(f.scope.clone(), spec).unwrap();
    let permit = f.capacity.clone().try_acquire_owned().unwrap();
    let started = plan.launch_in_pool(f.client.clone(), Arc::new(supplier), permit,
        f.admission()).await.unwrap();
    let report = probe_result(started).await;
    assert_eq!(material.0.load(Ordering::SeqCst), 1);
    assert_eq!(f.gate.stats().unwrap().active, 0);
    assert!(!f.gate.stats().unwrap().unconfirmed);
    assert_eq!(f.pool.snapshot().unwrap().residents[0].users, 0);
    report
}

#[path = "pool_context_tests.rs"]
mod pool_context;
