use super::{application::*, authorization::*, native_account::*, native_http::*};
use crate::proxy::Dialect;
use framer_core::ids::{AccountId, FramerUserId, ProjectId};
use framer_core::port::{CastleToken, Session};
use futures::{future::BoxFuture, FutureExt};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;
use tokio::time::{Duration, Instant};

#[tokio::test]
async fn account_cache_invalidation_preserves_unrelated_account_credentials() {
    let api = crate::HttpFramerClient::new();
    let replaced = AccountId(uuid::Uuid::new_v4());
    let unrelated = AccountId(uuid::Uuid::new_v4());
    for id in [replaced, unrelated, replaced] {
        api.ai_token.lock().unwrap().push((id, crate::auth::AiToken {
            bearer: "fixture-cache-token".to_owned(), expires_at_ms: i64::MAX,
        }));
    }
    api.invalidate_account_authorization(&replaced);
    let cache = api.ai_token.lock().unwrap();
    assert_eq!(cache.len(), 1);
    assert_eq!(cache[0].0, unrelated);
}

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

async fn prepared_call(client: &NativeAccountClient, scope: &AccountScope,
    source: Arc<Source>, raw: bool) -> (Box<dyn framer_core::native_dispatch::OwnedInvocation>, Arc<Semaphore>) {
    use crate::native_dispatch::{Binding, CreditEvidence, NativeDispatchProvider};
    use framer_core::{convert::ClientDialect, ids::TeamId, native_dispatch as port};
    use port::InvocationProvider;
    let deadline = Instant::now() + Duration::from_secs(30);
    let project = policy().project.clone();
    let team = TeamId("fixture-team".to_owned());
    let authorization = Arc::new(GenerationAuthorization::new(
        client.clone(), scope.clone(), source, policy()).unwrap());
    let provider = NativeDispatchProvider::new(Arc::new(crate::HttpFramerClient::new()),
        client.clone(), vec![Binding { scope: scope.clone(), team: team.clone(),
            project: project.clone(), authorization: Some(authorization),
            paid_credit: Some(CreditEvidence { available: 100.0, valid_until: deadline }) }],
        port::Policy { timeout: Duration::from_secs(30), max_body_bytes: 4096,
            missing_content_type: "application/json".to_owned(), chunk_tokens: 16,
            max_chunks: 1, continuation: None, allow_paid: true, credit_floor: 1.0 }).unwrap();
    let capacity = Arc::new(Semaphore::new(1));
    let permit = capacity.clone().acquire_owned().await.unwrap();
    let call = provider.prepare(port::Preparation {
        identity: port::Identity { account: scope.account(), user: scope.active_user().clone(),
            team, project, chat_session: "fixture-conversation".to_owned() },
        route: if raw { port::Route::Raw(ClientDialect::OpenAiResponses) } else { port::Route::Messages },
        deadline, max_total_tokens: 16,
    }, permit).await.unwrap();
    (call, capacity)
}

#[tokio::test]
async fn dropped_nonstreaming_result_holds_permit_until_authorization_worker_finishes() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client);
    let source = Source::new(true);
    let (call, capacity) = prepared_call(&client, &scope, source.clone(), false).await;
    let run = call.messages(json!({"model":"fixture-model", "stream":false,
        "messages":[{"role":"user", "content":"fixture"}]}));
    source.entered.acquire().await.unwrap().forget();
    drop(run.result);
    assert_eq!(capacity.available_permits(), 0);
    assert!(run.cleanup.clone().now_or_never().is_none());
    source.release.as_ref().unwrap().add_permits(1);
    run.cleanup.await.unwrap();
    assert_eq!(capacity.available_permits(), 1);
    client.close_account(&scope).unwrap().await.unwrap();
}

#[tokio::test]
async fn cancelled_raw_stream_holds_permit_until_authorization_worker_finishes() {
    use framer_core::native_dispatch::{FailureKind, RawStreamLimits};
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client);
    let source = Source::new(true);
    let (call, capacity) = prepared_call(&client, &scope, source.clone(), true).await;
    let run = call.raw_stream(json!({"model":"fixture-model", "stream":true, "input":"fixture"}),
        RawStreamLimits { retained_bytes: 4096, chunk_bytes: 1024, total_bytes: 8192, header_bytes: 1024 });
    source.entered.acquire().await.unwrap().forget();
    run.response.cancellation.cancel();
    assert_eq!(run.response.result.await.err().unwrap().kind, FailureKind::Cancelled);
    assert_eq!(capacity.available_permits(), 0);
    assert!(run.response.cleanup.clone().now_or_never().is_none());
    source.release.as_ref().unwrap().add_permits(1);
    run.response.cleanup.await.unwrap();
    assert_eq!(capacity.available_permits(), 1);
    client.close_account(&scope).unwrap().await.unwrap();
}

// Proposal only, uncompiled and unrun. Append in a NEW combined revision
// to experimental_native/authorization_tests.rs. Requires the HTTP R2
// composed API plus auth observed-preparation hooks. Preserve old test bodies.
// Reuses Source, open(), prepared_call() and that module's existing imports.

fn pending_auth_composed_options() -> framer_core::native_dispatch::ComposedStreamOptions {
    use framer_core::{convert::ClientDialect, native_dispatch::{ComposedStreamLimits, ComposedStreamOptions}};
    ComposedStreamOptions { dialect: ClientDialect::AnthropicMessages, include_usage: true,
        limits: ComposedStreamLimits { frame_bytes: 8192, retained_bytes: 16384,
            retained_frames: 2, state_bytes: 64 * 1024 * 1024, json_bytes: 16384,
            source_chunk_bytes: 5, header_bytes: 4096, total_bytes: 1024 * 1024,
            terminal_bytes: 1024 } }
}

async fn pending_auth_composed_cleanup(drop_header: bool) {
    use framer_core::native_dispatch::{ComposedStreamRun, FailureKind, InvocationRun};
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client);
    let source = Source::new(true);
    let (call, capacity) = prepared_call(&client, &scope, source.clone(), false).await;
    let ComposedStreamRun { response, finished } = call.composed_stream_for_client(
        json!({"model":"fixture-model", "stream":false,
            "messages":[{"role":"user", "content":"fixture"}]}),
        pending_auth_composed_options(), "fixture-client-alias".to_owned());
    let InvocationRun { result, cleanup, cancellation, .. } = response;
    tokio::time::timeout(Duration::from_secs(2), source.entered.acquire())
        .await.unwrap().unwrap().forget();
    assert_eq!(source.sessions.lock().unwrap().len(), 1);
    assert_eq!(capacity.available_permits(), 0);
    if drop_header {
        drop(result);
    } else {
        cancellation.cancel();
        let error = tokio::time::timeout(Duration::from_secs(2), result)
            .await.unwrap().err().unwrap();
        assert_eq!(error.kind, FailureKind::Cancelled);
    }
    let terminal = tokio::time::timeout(Duration::from_secs(2), finished).await.unwrap();
    assert_eq!(terminal.result, Err(FailureKind::Cancelled));
    assert_eq!(terminal.emitted_bytes, 0);
    assert!(terminal.attempts.is_empty());
    // This await gives an incorrect early-cleanup implementation an opportunity
    // to complete. Merely sampling immediately after cancellation is weaker.
    assert!(tokio::time::timeout(Duration::from_millis(25), cleanup.clone()).await.is_err());
    assert_eq!(capacity.available_permits(), 0);
    source.release.as_ref().unwrap().add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), cleanup).await.unwrap().unwrap();
    assert_eq!(capacity.available_permits(), 1);
    tokio::time::timeout(Duration::from_secs(2), client.close_account(&scope).unwrap())
        .await.unwrap().unwrap();
}

#[tokio::test]
async fn cancelled_composed_stream_holds_permit_until_authorization_worker_finishes() {
    pending_auth_composed_cleanup(false).await;
}

#[tokio::test]
async fn abandoned_composed_header_holds_permit_until_authorization_worker_finishes() {
    pending_auth_composed_cleanup(true).await;
}
