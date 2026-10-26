//! Generation-bound authorization. No payload rewriting, alternate category,
//! refusal retry, or implicit credential source.
use crate::request::invocation::AttemptAuthorization;
use crate::request::invocation::AttemptContext;
use crate::request::invocation::AuthorizedAttempt;
use crate::request::invocation::PreparedAttempt;
use crate::account::client::AccountScope;
use crate::account::client::NativeAccountClient;
use crate::account::client::PreparationControl;
use crate::account::client::PreparationFailure;
use crate::account::client::PreparationResult;
use crate::account::client::SuppliedAuth;
use crate::transport::http::FreshCastleToken;
use framer_core::ids::ProjectId;
use framer_core::port::BrowserBridge;
use framer_core::port::Session;
use futures::future::BoxFuture;
use futures::FutureExt;
use serde_json::Value;
use std::sync::Arc;


#[path = "authorization_context.rs"]
mod context_binding;
pub use context_binding::BoundAttemptContext;
use context_binding::AuthorizationContext;

#[path = "credential_target.rs"]
mod credential_target;
pub use credential_target::CredentialTarget;
pub use credential_target::CurrentRequestCookies;

pub(crate) const MAX_COOKIE_HEADER_BYTES: usize = 16 * 1024;

pub struct CredentialMaterial { pub bearer: String, pub castle: FreshCastleToken }

pub struct PreparedCredentials {
    pub material: CredentialMaterial,
    pub current_cookies: Option<CurrentRequestCookies>,
}
impl From<CredentialMaterial> for PreparedCredentials {
    fn from(material: CredentialMaterial) -> Self {
        Self { material, current_cookies: None }
    }
}

/// The source must own its work until completion. If external work may outlive
/// an error, return unconfirmed so the account remains closed for mutation.
pub trait CredentialSource: Send + Sync {
    fn issue<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        control: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>>;

    /// Exact target from the prepared request, forwarded by owned wrappers.
    fn issue_for_target<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        _target: &'a CredentialTarget, control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<PreparedCredentials>> {
        async move {
            let result = self.issue(scope, session, control).await;
            PreparationResult { value: result.value.map(PreparedCredentials::from),
                cleanup_confirmed: result.cleanup_confirmed }
        }.boxed()
    }
}

/// Explicit metadata from the actual feature. There are no category defaults.
/// Prompt clipping affects only the display label, never request/history bytes.
pub struct ContextPolicy {
    pub project: ProjectId,
    pub feature_category: String,
    pub feature_name: String,
    pub step: String,
    pub display_prompt_chars: usize,
}
impl ContextPolicy {
    fn validate(&self) -> bool {
        !self.project.0.is_empty() && self.display_prompt_chars <= 1024
            && [&self.feature_category, &self.feature_name, &self.step]
                .iter().all(|value| !value.is_empty() && value.len() <= 256)
    }
    fn context(&self, attempt: &PreparedAttempt<'_>) -> Result<AttemptContext, String> {
        if attempt.project != &self.project { return Err("authorization project mismatch".to_owned()); }
        let payload = attempt.payload;
        let item = payload["messages"].as_array().or_else(|| payload["input"].as_array())
            .and_then(|items| items.iter().rev().find(|item| item["role"] == "user"));
        let content = item.map(|item| &item["content"]).unwrap_or(&payload["input"]);
        let mut prompt = String::new();
        let mut remaining = self.display_prompt_chars;
        let mut append = |text: &str| {
            for c in text.chars().take(remaining) { prompt.push(c); remaining -= 1; }
        };
        match content {
            Value::String(text) => append(text),
            Value::Array(blocks) => for block in blocks {
                if let Some(text) = block["text"].as_str().or_else(|| block["input_text"].as_str()) {
                    append(text);
                }
            },
            _ => {}
        }
        Ok(AttemptContext {
            feature_category: self.feature_category.clone(), feature_name: self.feature_name.clone(),
            step: self.step.clone(), turn_id: uuid::Uuid::new_v4().to_string(), user_prompt: prompt,
        })
    }
}

pub struct GenerationAuthorization {
    client: NativeAccountClient,
    scope: AccountScope,
    source: Arc<dyn CredentialSource>,
    context: AuthorizationContext,
}
impl GenerationAuthorization {
    pub fn new(client: NativeAccountClient, scope: AccountScope,
        source: Arc<dyn CredentialSource>, context: ContextPolicy) -> Result<Self, String> {
        if !context.validate() { return Err("invalid explicit authorization context".to_owned()); }
        Ok(Self { client, scope, source, context: AuthorizationContext::Derived(context) })
    }

    /// Preserve the actual application's context for one conversation step.
    /// This neither generates a turn ID nor clips or derives the display prompt.
    /// The credential source and observed preparation ownership are unchanged.
    pub fn for_attempt(client: NativeAccountClient, scope: AccountScope,
        source: Arc<dyn CredentialSource>, context: BoundAttemptContext) -> Result<Self, String> {
        let context = AuthorizationContext::bound(context)?;
        Ok(Self { client, scope, source, context })
    }

    pub(crate) fn for_sequence(client: NativeAccountClient, scope: AccountScope,
        source: Arc<dyn CredentialSource>, contexts: Vec<BoundAttemptContext>) -> Result<Self, String> {
        let context = AuthorizationContext::sequence(contexts)?;
        Ok(Self { client, scope, source, context })
    }
}
impl GenerationAuthorization {
    fn prepare<'a>(&'a self, attempt: PreparedAttempt<'a>,
        observer: Option<&'a dyn crate::request::invocation::PreparationObserver>)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        async move {
            if !self.scope.same_generation(attempt.scope) {
                return Err("authorization generation mismatch".to_owned());
            }
            let context = self.context.context(&attempt)?;
            let target = CredentialTarget { project: attempt.project.clone(),
                path: framer_api::routes::by_dialect(attempt.dialect).path };
            let scope = self.scope.clone();
            let source = self.source.clone();
            let pending = self.client.prepare_owned(&self.scope, attempt.deadline,
                move |session, control| async move {
                    let issued = source.issue_for_target(&scope, &session, &target, &control).await;
                    let value = issued.value.and_then(|prepared| {
                        let material = prepared.material;
                        if material.bearer.is_empty() || material.bearer.len() > 16 * 1024 {
                            return Err(PreparationFailure::Rejected);
                        }
                        Ok(AuthorizedAttempt {
                            auth: SuppliedAuth::new(&scope, material.bearer, material.castle)
                                .with_current_cookies(prepared.current_cookies), context,
                        })
                    });
                    PreparationResult { value, cleanup_confirmed: issued.cleanup_confirmed }
                }).map_err(|_| "authorization admission failed".to_owned())?;
            if let Some(observer) = observer {
                observer.preparing(pending.completion(), pending.cancellation());
            }
            pending.await.map_err(|error| format!("native authorization: {error:?}"))
        }.boxed()
    }
}
impl AttemptAuthorization for GenerationAuthorization {
    fn authorize<'a>(&'a self, attempt: PreparedAttempt<'a>)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        self.prepare(attempt, None)
    }
    fn authorize_observed<'a>(&'a self, attempt: PreparedAttempt<'a>,
        observer: &'a dyn crate::request::invocation::PreparationObserver)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        self.prepare(attempt, Some(observer))
    }
}

pub mod control;
pub(crate) mod source;
#[cfg(test)]
mod authorization_tests;
