use super::*;
use crate::request::invocation::AttemptAuthorization;
use crate::authorization::CredentialMaterial;
use crate::authorization::CredentialSource;
use crate::authorization::GenerationAuthorization;
use crate::account::client::AccountScope;
use crate::account::client::CloseObserver;
use crate::account::client::NativeAccountClient;
use crate::account::client::PreparationControl;
use crate::account::client::PreparationResult;
use crate::transport::http::FreshCastleToken;
use crate::transport::http::NativeEgress;
use crate::transport::http::NativeHttpLimits;
use framer_api::proxy::Dialect;
use framer_core::ids::AccountId;
use framer_core::ids::FramerUserId;
use framer_core::port::CastleToken;
use framer_core::port::Session;
use futures::future::BoxFuture;
use futures::FutureExt;
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tokio::time::Duration;
use tokio::time::Instant;

fn open(client: &NativeAccountClient) -> AccountScope {
    client.open_account(AccountId(uuid::Uuid::new_v4()), Session {
        active_user: FramerUserId("fixture-user".into()), cookie_header: "session=fixture".into(),
    }, NativeHttpLimits::default(), NativeEgress::Direct).unwrap()
}
fn binding() -> BoundAttemptContext {
    BoundAttemptContext {
        project: ProjectId("fixture-project".into()), chat_session: "fixture-thread".into(),
        step_index: 2, context: AttemptContext {
            feature_category: "fixture-category".into(), feature_name: "fixture-feature".into(),
            step: "fixture-step".into(), turn_id: "actual-fixture-turn".into(),
            user_prompt: "Full prompt 🚀é\nincluding its supplied label".into(),
        },
    }
}
fn prepared<'a>(scope: &'a AccountScope, project: &'a ProjectId, payload: &'a Value)
    -> PreparedAttempt<'a> {
    PreparedAttempt { scope, project, payload, dialect: Dialect::OpenRouterResponses,
        chat_session: "fixture-thread", step_index: 2,
        deadline: Instant::now() + Duration::from_secs(20) }
}
fn fields(value: &AttemptContext) -> [&str; 5] {
    [&value.feature_category, &value.feature_name, &value.step, &value.turn_id, &value.user_prompt]
}

#[tokio::test]
async fn supplied_context_cancelled_waiter_retains_observed_preparation() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client);
    let source = Source::new(true);
    let supplier = GenerationAuthorization::for_attempt(client.clone(), scope.clone(),
        source.clone(), binding()).unwrap();
    let project = binding().project;
    let payload = json!({"model":"fixture-model", "input":"fixture"});
    let observed = Mutex::new(None);
    let observer = |cleanup: CloseObserver, cancel: PreparationControl| {
        *observed.lock().unwrap() = Some((cleanup, cancel));
    };
    let mut waiting = supplier.authorize_observed(prepared(&scope, &project, &payload), &observer);
    assert!(waiting.as_mut().now_or_never().is_none());
    assert!(observed.lock().unwrap().is_some());
    timeout(Duration::from_secs(2), source.entered.acquire()).await.unwrap().unwrap().forget();
    drop(waiting);
    let (cleanup, cancel) = observed.into_inner().unwrap().unwrap();
    assert!(cancel.is_cancelled());
    let closed = client.close_account(&scope).unwrap();
    assert!(timeout(Duration::from_millis(25), cleanup.clone()).await.is_err());
    assert!(timeout(Duration::from_millis(25), closed.clone()).await.is_err());
    source.release.as_ref().unwrap().add_permits(1);
    timeout(Duration::from_secs(2), cleanup).await.unwrap().unwrap();
    timeout(Duration::from_secs(2), closed).await.unwrap().unwrap();
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn supplied_context_rejects_replacement_generation_before_credentials() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client);
    let source = Source::new(false);
    let supplier = GenerationAuthorization::for_attempt(client.clone(), scope.clone(),
        source.clone(), binding()).unwrap();
    client.close_account(&scope).unwrap().await.unwrap();
    let next = client.open_account(scope.account(), Session {
        active_user: scope.active_user().clone(), cookie_header: "session=replacement".into(),
    }, NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    let project = binding().project;
    assert!(supplier.authorize(prepared(&next, &project, &json!({"input":"fixture"})))
        .await.is_err());
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    client.close_account(&next).unwrap().await.unwrap();
}

#[tokio::test]
async fn supplied_context_rejects_empty_or_oversized_binding() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client);
    let source = Source::new(false);
    let invalid_bindings: [(&str, fn(&mut BoundAttemptContext)); 7] = [
        ("empty project", |value| value.project.0.clear()),
        ("empty conversation", |value| value.chat_session.clear()),
        ("empty feature category", |value| value.context.feature_category.clear()),
        ("empty feature name", |value| value.context.feature_name.clear()),
        ("empty step", |value| value.context.step.clear()),
        ("empty turn", |value| value.context.turn_id.clear()),
        ("oversized prompt", |value| value.context.user_prompt = "x".repeat(4097)),
    ];
    for (name, invalidate) in invalid_bindings {
        let mut value = binding();
        invalidate(&mut value);
        assert!(GenerationAuthorization::for_attempt(client.clone(), scope.clone(),
            source.clone(), value).is_err(), "{name}");
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    client.close_account(&scope).unwrap().await.unwrap();
}

#[tokio::test]
async fn supplied_context_preserves_every_field_and_payload() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client);
    let source = Source::new(false);
    let expected = binding();
    let supplier = GenerationAuthorization::for_attempt(client.clone(), scope.clone(),
        source.clone(), binding()).unwrap();
    let payload = json!({"model":"fixture-alias", "stream":false,
        "reasoning":{"effort":"max"}, "input":[
            {"role":"system", "content":"preserved system"},
            {"role":"user", "content":"different from the supplied display label"}],
        "tools":[{"type":"function", "name":"preserve", "parameters":{"type":"object"}}]});
    let before = payload.clone();
    for _ in 0..2 {
        let got = supplier.authorize(prepared(&scope, &expected.project, &payload)).await.unwrap();
        assert_eq!(fields(&got.context), fields(&expected.context));
        assert_eq!(payload, before);
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    client.close_account(&scope).unwrap().await.unwrap();
}

#[tokio::test]
async fn supplied_context_rejects_wrong_project_conversation_and_step_before_credentials() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client);
    let source = Source::new(false);
    let supplier = GenerationAuthorization::for_attempt(client.clone(), scope.clone(),
        source.clone(), binding()).unwrap();
    let payload = json!({"input":"fixture"});
    let project = binding().project;
    let wrong = ProjectId("other-project".into());
    let requests = [
        ("wrong project", PreparedAttempt {
            project: &wrong, ..prepared(&scope, &project, &payload)
        }),
        ("wrong conversation", PreparedAttempt {
            chat_session: "other-thread", ..prepared(&scope, &project, &payload)
        }),
        ("wrong step", PreparedAttempt {
            step_index: 3, ..prepared(&scope, &project, &payload)
        }),
    ];
    for (name, request) in requests {
        assert!(supplier.authorize(request).await.is_err(), "{name}");
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    client.close_account(&scope).unwrap().await.unwrap();
}

struct Source {
    calls: AtomicUsize,
    entered: Semaphore,
    release: Option<Semaphore>,
}
impl Source {
    fn new(held: bool) -> Arc<Self> {
        Arc::new(Self { calls: AtomicUsize::new(0), entered: Semaphore::new(0),
            release: held.then(|| Semaphore::new(0)) })
    }
}
impl CredentialSource for Source {
    fn issue<'a>(&'a self, scope: &'a AccountScope, _: &'a Session,
        _: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.add_permits(1);
            if let Some(release) = &self.release { release.acquire().await.unwrap().forget(); }
            PreparationResult::finished(Ok(CredentialMaterial {
                bearer: "fixture-bearer".to_owned(),
                castle: FreshCastleToken::from_browser(scope.account(),
                    CastleToken("fixture-castle".to_owned())),
            }))
        }.boxed()
    }
}
