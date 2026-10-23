use super::decode;
use framer_api::{HttpFramerClient, native_dispatch::{Binding, NativeDispatchProvider}};
use framer_api::experimental_native::{
    application::{AttemptAuthorization, AuthorizedAttempt, PreparedAttempt},
    native_account::NativeAccountClient,
    native_http::{NativeEgress, NativeHttpLimits},
};
use framer_core::{convert::ClientDialect, ids::{AccountId, FramerUserId, ProjectId, TeamId},
    native_dispatch::{ComposedStreamOptions, FailureKind, Identity, InvocationProvider,
        Preparation, Route}, port::Session};
use futures::future::BoxFuture;
use serde_json::{json, Value};
use std::{sync::{Arc, atomic::{AtomicUsize, Ordering}}, time::Duration};
use tokio::{sync::Semaphore, time::Instant};

#[derive(Default)]
struct RefuseBeforeNetwork(AtomicUsize);
impl AttemptAuthorization for RefuseBeforeNetwork {
    fn authorize<'a>(&'a self, _: PreparedAttempt<'a>)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err("synthetic refusal before any credentials or HTTP".into())
        })
    }
}

async fn exercise(old_budget: bool) {
    let mut config: Value = serde_json::from_str(include_str!(
        "../../../examples/browserless-routing.json")).unwrap();
    if old_budget { config["streaming"]["composed"]["state_bytes"] = json!(65536); }
    let (policy, streaming) = decode(config.to_string().as_bytes()).unwrap();
    let client = NativeAccountClient::new().unwrap();
    let account = AccountId::new();
    let user = FramerUserId("fixture-user".into());
    let scope = client.open_account(account, Session { active_user: user.clone(),
        cookie_header: "session=fixture-only".into() }, NativeHttpLimits::default(),
        NativeEgress::Direct).unwrap();
    let supplier = Arc::new(RefuseBeforeNetwork::default());
    let team = TeamId("fixture-team".into());
    let project = ProjectId("fixture-project".into());
    let binding = Binding { scope: scope.clone(), team: team.clone(), project: project.clone(),
        authorization: Some(supplier.clone()), paid_credit: None };
    let provider = NativeDispatchProvider::new(Arc::new(HttpFramerClient::new()),
        client.clone(), vec![binding], policy).unwrap();
    let capacity = Arc::new(Semaphore::new(1));
    let call = provider.prepare(Preparation {
        identity: Identity { account, user, team, project, chat_session: "fixture-chat".into() },
        route: Route::Responses, deadline: Instant::now() + Duration::from_secs(5),
        max_total_tokens: 512,
    }, capacity.clone().try_acquire_owned().unwrap()).await.unwrap();
    let run = call.composed_stream_for_client(json!({"model":"gpt-4.1", "stream":false,
        "input":"Count from one to five hundred in words.", "max_output_tokens":512}),
        ComposedStreamOptions { dialect: ClientDialect::OpenAiResponses, include_usage: false,
            limits: streaming.unwrap().composed }, "gpt-4.1".into());
    let expected = if old_budget { FailureKind::BodyTooLarge } else { FailureKind::Authorization };
    assert_eq!(run.response.result.await.err().unwrap().kind, expected);
    assert_eq!(run.finished.await.result, Err(expected));
    assert_eq!(run.response.cleanup.await, Ok(()));
    assert_eq!(supplier.0.load(Ordering::SeqCst), usize::from(!old_budget));
    assert_eq!(capacity.available_permits(), 1);
    drop(provider);
    assert_eq!(client.close_account(&scope).unwrap().await, Ok(()));
    client.shutdown_idle().await.unwrap();
}

#[tokio::test]
async fn browserless_example_budget_reaches_authorization_without_network() {
    tokio::time::timeout(Duration::from_secs(5), exercise(false)).await.unwrap();
}

#[tokio::test]
async fn original_budget_refuses_before_authorization_with_confirmed_cleanup() {
    tokio::time::timeout(Duration::from_secs(5), exercise(true)).await.unwrap();
}
