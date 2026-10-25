use crate::request::invocation::*;
use crate::authorization::*;
use crate::account::client::*;
use crate::transport::http::*;
use framer_api::proxy::Dialect;
use framer_core::ids::AccountId;
use framer_core::ids::FramerUserId;
use framer_core::ids::ProjectId;
use framer_core::port::CastleToken;
use framer_core::port::Session;
use futures::future::BoxFuture;
use futures::FutureExt;
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::Semaphore;
use tokio::time::Duration;
use tokio::time::Instant;


struct Source {
    sessions: Mutex<Vec<(AccountId, String)>>,
    entered: Semaphore,
    release: Option<Arc<Semaphore>>,
}
impl Source {
    fn new(held: bool) -> Arc<Self> {
        Arc::new(Self { sessions: Mutex::new(Vec::new()), entered: Semaphore::new(0),
            release: held.then(|| Arc::new(Semaphore::new(0))) })
    }
}
impl CredentialSource for Source {
    fn issue<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        _: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async move {
            self.sessions.lock().unwrap().push((scope.account(), session.cookie_header.clone()));
            self.entered.add_permits(1);
            if let Some(release) = &self.release { release.acquire().await.unwrap().forget(); }
            PreparationResult::finished(Ok(CredentialMaterial {
                bearer: "fixture-bearer".to_owned(),
                castle: FreshCastleToken::from_browser(scope.account(), CastleToken("fixture-castle".to_owned())),
            }))
        }.boxed()
    }
}
fn open(client: &NativeAccountClient) -> AccountScope {
    client.open_account(AccountId(uuid::Uuid::new_v4()), Session {
        active_user: FramerUserId("fixture-user".to_owned()),
        cookie_header: "session=fixture-exact-generation".to_owned(),
    }, NativeHttpLimits::default(), NativeEgress::Direct).unwrap()
}
fn policy() -> ContextPolicy {
    ContextPolicy { project: ProjectId("fixture-project".to_owned()),
        feature_category: "fixture-category".to_owned(), feature_name: "fixture-feature".to_owned(),
        step: "fixture-step".to_owned(), display_prompt_chars: 3 }
}
fn attempt<'a>(scope: &'a AccountScope, project: &'a ProjectId, payload: &'a Value) -> PreparedAttempt<'a> {
    PreparedAttempt { scope, project, dialect: Dialect::OpenAiResponses, payload,
        chat_session: "fixture-conversation", step_index: 2,
        deadline: Instant::now() + Duration::from_secs(30) }
}

#[tokio::test]
async fn source_uses_exact_generation_and_keeps_request_fields_unchanged() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client);
    let source = Source::new(false);
    let project = policy().project.clone();
    let supplier = GenerationAuthorization::new(client.clone(), scope.clone(), source.clone(), policy()).unwrap();
    let payload = json!({"model":"fixture-model", "reasoning":{"effort":"max"},
        "input":[{"role":"user", "content":[{"type":"input_text", "text":"a🚀éz"}]}],
        "tools":[{"type":"function", "name":"keep", "parameters":{"type":"object"}}]});
    let before = payload.clone();
    let authorized = supplier.authorize(attempt(&scope, &project, &payload)).await.unwrap();
    assert_eq!(payload, before);
    assert_eq!(authorized.context.user_prompt, "a🚀é");
    assert_eq!(authorized.context.feature_category, "fixture-category");
    assert_eq!(authorized.context.feature_name, "fixture-feature");
    assert_eq!(authorized.context.step, "fixture-step");
    assert_eq!(source.sessions.lock().unwrap().as_slice(),
        &[(scope.account(), "session=fixture-exact-generation".to_owned())]);
    client.close_account(&scope).unwrap().await.unwrap();
}

#[tokio::test]
async fn observer_precedes_suspension_and_owns_cancelled_authorization_cleanup() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client);
    let source = Source::new(true);
    let project = policy().project.clone();
    let supplier = GenerationAuthorization::new(client.clone(), scope.clone(), source.clone(), policy()).unwrap();
    let payload = json!({"model":"fixture-model", "input":"fixture"});
    let observed = Mutex::new(None);
    let observer = |cleanup, cancel| { *observed.lock().unwrap() = Some((cleanup, cancel)); };
    let mut future = supplier.authorize_observed(attempt(&scope, &project, &payload), &observer);
    assert!(future.as_mut().now_or_never().is_none());
    assert!(observed.lock().unwrap().is_some());
    source.entered.acquire().await.unwrap().forget();
    drop(future);
    let (cleanup, cancel): (CloseObserver, PreparationControl) = observed.into_inner().unwrap().unwrap();
    assert!(cancel.is_cancelled());
    assert!(cleanup.clone().now_or_never().is_none());
    let mut change = client.begin_session_change(scope.active_user()).unwrap();
    assert!(change.ready().now_or_never().is_none());
    source.release.as_ref().unwrap().add_permits(1);
    cleanup.await.unwrap();
    change.ready().await.unwrap();
    client.finish_session_change(&mut change).unwrap();
}

#[tokio::test]
async fn project_or_generation_mismatch_never_reaches_credential_source() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client);
    let other = open(&client);
    let source = Source::new(false);
    let project = policy().project.clone();
    let supplier = GenerationAuthorization::new(client.clone(), scope.clone(), source.clone(), policy()).unwrap();
    let payload = json!({"model":"fixture-model", "input":"fixture"});
    assert!(supplier.authorize(attempt(&other, &project, &payload)).await.is_err());
    assert!(supplier.authorize(attempt(&scope, &ProjectId("wrong-project".to_owned()), &payload)).await.is_err());
    client.close_account(&scope).unwrap().await.unwrap();
    let next = client.open_account(scope.account(), Session {
        active_user: scope.active_user().clone(), cookie_header: "session=fixture-replaced".to_owned(),
    }, NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    assert!(supplier.authorize(attempt(&next, &project, &payload)).await.is_err());
    assert!(source.sessions.lock().unwrap().is_empty());
    client.close_account(&next).unwrap().await.unwrap();
    client.close_account(&other).unwrap().await.unwrap();
}
