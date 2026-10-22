use super::*;
use framer_api::experimental_native::{authorization::CredentialMaterial,
    native_account::{CloseObserver, PreparationControl, PreparationResult},
    native_http::{FreshCastleToken, NativeEgress, NativeHttpLimits}};
use framer_core::{ids::{AccountId, FramerUserId, ProjectId, TeamId}, port::{CastleToken, Session}};
use std::sync::{Mutex, atomic::{AtomicUsize, Ordering}};

struct Source(AtomicUsize);
impl CredentialSource for Source {
    fn issue<'a>(&'a self, scope: &'a AccountScope, _: &'a Session, _: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            PreparationResult::finished(Ok(CredentialMaterial {
                bearer: "fixture-bearer".into(),
                castle: FreshCastleToken::from_browser(scope.account(), CastleToken("fixture-token".into())),
            }))
        }.boxed()
    }
}

fn fixture() -> (Authorization, Arc<Source>) {
    let native = NativeAccountClient::new().unwrap();
    let user = FramerUserId("fixture-user".into());
    let scope = native.open_account(AccountId::new(), Session { active_user: user.clone(),
        cookie_header: "session=fixture".into() }, NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    let source = Arc::new(Source(AtomicUsize::new(0)));
    let identity = Identity { account: scope.account(), user, team: TeamId("fixture-team".into()),
        project: ProjectId("fixture-project".into()), chat_session: "fixture-thread".into() };
    let contexts = ["first", "next"].into_iter().map(|step| AttemptContext {
        feature_category: "actual-category".into(), feature_name: "actual-feature".into(),
        step: step.into(), turn_id: format!("actual-turn-{step}"),
        user_prompt: format!("exact supplied annotation {step}\n🚀"),
    }).collect();
    (Authorization { native, scope, source: source.clone(), identity, contexts }, source)
}

#[tokio::test]
async fn routing_authorization_preserves_each_supplied_continuation_context() {
    let (auth, source) = fixture();
    let payload = serde_json::json!({"model":"fixture-model","input":"unchanged history"});
    let observed = Mutex::new(Vec::new());
    let observer = |done: CloseObserver, _: PreparationControl| {
        observed.lock().unwrap().push(done);
    };
    for index in 0..2 {
        let result = auth.authorize_observed(PreparedAttempt {
            scope: &auth.scope, project: &auth.identity.project,
            dialect: framer_api::proxy::Dialect::OpenRouterResponses, payload: &payload,
            chat_session: &auth.identity.chat_session, step_index: index,
            deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(2),
        }, &observer).await.unwrap();
        assert_eq!(result.context.turn_id, auth.contexts[index as usize].turn_id);
        assert_eq!(result.context.step, auth.contexts[index as usize].step);
        assert_eq!(result.context.user_prompt, auth.contexts[index as usize].user_prompt);
    }
    assert_eq!(source.0.load(Ordering::SeqCst), 2);
    let observed = observed.into_inner().unwrap();
    assert_eq!(observed.len(), 2);
    for cleanup in observed { assert_eq!(cleanup.await, Ok(())); }
    auth.native.close_account(&auth.scope).unwrap().await.unwrap();
}

#[tokio::test]
async fn routing_unknown_step_or_conversation_never_issues_credentials() {
    let (auth, source) = fixture();
    let payload = serde_json::json!({"model":"fixture-model"});
    for (index, conversation) in [(2, "fixture-thread"), (0, "different-thread")] {
        assert!(auth.authorize(PreparedAttempt {
            scope: &auth.scope, project: &auth.identity.project,
            dialect: framer_api::proxy::Dialect::OpenRouterResponses, payload: &payload,
            chat_session: conversation, step_index: index,
            deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(2),
        }).await.is_err());
    }
    assert_eq!(source.0.load(Ordering::SeqCst), 0);
    auth.native.close_account(&auth.scope).unwrap().await.unwrap();
}
