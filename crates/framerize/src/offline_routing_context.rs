use super::*;
use framer_api::experimental_native::{application::{AttemptAuthorization,
    AuthorizedAttempt, PreparedAttempt, PreparationObserver},
    authorization::{BoundAttemptContext, CredentialSource, CredentialTarget, GenerationAuthorization},
    control_authorization::OfflineMintTarget,
    native_account::{AccountScope, NativeAccountClient}};

fn valid(text: &str) -> bool {
    !text.is_empty() && text.len() <= 256 && !text.chars().any(char::is_control)
}

pub(super) fn path(route: port::Route) -> &'static str {
    use framer_api::{proxy::Dialect, routes};
    use framer_core::convert::ClientDialect;
    routes::by_dialect(match route {
        port::Route::Responses => Dialect::OpenRouterResponses,
        port::Route::Messages => Dialect::OpenRouterMessages,
        port::Route::Raw(ClientDialect::OpenAiResponses) => Dialect::OpenAiResponses,
        port::Route::Raw(ClientDialect::AnthropicMessages) => Dialect::AnthropicMessages,
        port::Route::Raw(ClientDialect::OpenAiChat) => Dialect::OpenRouterChat,
    }).path
}

pub(super) struct Authorization {
    pub native: NativeAccountClient,
    pub scope: AccountScope,
    pub source: Arc<dyn CredentialSource>,
    pub identity: Identity,
    pub contexts: Vec<AttemptContext>,
}

impl Authorization {
    fn for_attempt(&self, index: u32) -> Result<GenerationAuthorization, String> {
        let c = self.contexts.get(index as usize).ok_or("missing explicit attempt context")?;
        GenerationAuthorization::for_attempt(self.native.clone(), self.scope.clone(),
            self.source.clone(), BoundAttemptContext {
                project: self.identity.project.clone(), chat_session: self.identity.chat_session.clone(),
                step_index: index, context: AttemptContext {
                    feature_category: c.feature_category.clone(), feature_name: c.feature_name.clone(),
                    step: c.step.clone(), turn_id: c.turn_id.clone(), user_prompt: c.user_prompt.clone(),
                },
            })
    }
}

impl AttemptAuthorization for Authorization {
    fn authorize<'a>(&'a self, attempt: PreparedAttempt<'a>)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        async move {
            self.for_attempt(attempt.step_index)?.authorize(attempt).await
        }.boxed()
    }

    fn authorize_observed<'a>(&'a self, attempt: PreparedAttempt<'a>,
        observer: &'a dyn PreparationObserver)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        async move {
            self.for_attempt(attempt.step_index)?.authorize_observed(attempt, observer).await
        }.boxed()
    }
}

#[cfg(test)]
#[path = "offline_routing_context_tests.rs"]
mod tests;

pub(super) fn resolve(shared: &Shared, request: &Preparation, payload: &Value)
    -> Result<(RequestContext, Arc<dyn SourceBuilder>), DispatchError> {
    let mut context = shared.contexts.resolve(request, payload)?;
    // Only the owned ledger read may provide paid credit evidence.
    context.paid_credit = None;
    let count = if matches!(request.route, port::Route::Raw(_)) { 1 }
        else { shared.policy.max_chunks as usize };
    if context.identity != request.identity || context.attempts.len() != count
        || [&context.identity.user.0, &context.identity.team.0,
            &context.identity.project.0, &context.identity.chat_session].iter().any(|s| !valid(s))
        || context.attempts.iter().any(|c| [&c.feature_category, &c.feature_name,
            &c.step, &c.turn_id].iter().any(|s| !valid(s)) || c.user_prompt.len() > 4096) {
        return Err(DispatchError::new(FailureKind::MissingContext));
    }
    let target = OfflineMintTarget::new(CredentialTarget {
        project: request.identity.project.clone(), path: path(request.route),
    }, context.page_url.clone()).map_err(|_| DispatchError::new(FailureKind::InvalidScope))?;
    if matches!(request.route, port::Route::Raw(_)) {
        if !shared.policy.allow_paid { return Err(DispatchError::new(FailureKind::PaidDisabled)); }
    }
    Ok((context, (shared.sources)(target)))
}
