//! Generation-bound authorization. No payload rewriting, alternate category,
//! refusal retry, or implicit credential source.
use super::application::{AttemptAuthorization, AttemptContext, AuthorizedAttempt, PreparedAttempt};
use super::native_account::{AccountScope, NativeAccountClient, PreparationControl,
    PreparationFailure, PreparationResult, SuppliedAuth};
use super::native_http::FreshCastleToken;
use framer_core::ids::ProjectId;
use framer_core::port::{BrowserBridge, Session};
use futures::{future::BoxFuture, FutureExt};
use serde_json::Value;
use std::sync::Arc;

#[cfg(test)]
#[path = "supplier_context_tests.rs"]
mod supplier_context_tests;

#[path = "authorization_context.rs"]
mod context_binding;
pub use context_binding::BoundAttemptContext;
use context_binding::AuthorizationContext;

#[path = "credential_target.rs"]
mod credential_target;
pub use credential_target::{CredentialTarget, CurrentRequestCookies};

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
}
impl GenerationAuthorization {
    fn prepare<'a>(&'a self, attempt: PreparedAttempt<'a>,
        observer: Option<&'a dyn super::application::PreparationObserver>)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        async move {
            if !self.scope.same_generation(attempt.scope) {
                return Err("authorization generation mismatch".to_owned());
            }
            let context = self.context.context(&attempt)?;
            let target = CredentialTarget { project: attempt.project.clone(),
                path: crate::routes::by_dialect(attempt.dialect).path };
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
        observer: &'a dyn super::application::PreparationObserver)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        self.prepare(attempt, Some(observer))
    }
}
/// Adapter for the established private auth boundary and the account's own
/// browser token runtime. This is a control adapter, not proof of browserless
/// acceptance. A separately validated SDK runtime may implement CredentialSource.
pub struct BrowserSessionSource {
    api: Arc<crate::HttpFramerClient>,
    browser: Arc<dyn BrowserBridge>,
}
impl BrowserSessionSource {
    pub fn new(api: Arc<crate::HttpFramerClient>, browser: Arc<dyn BrowserBridge>) -> Self {
        Self { api, browser }
    }
}
impl CredentialSource for BrowserSessionSource {
    fn issue<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        control: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async move {
            if scope.active_user() != &session.active_user || control.is_cancelled() {
                return PreparationResult::finished(Err(PreparationFailure::Cancelled));
            }
            let bearer = match crate::auth::ai_access_token(&self.api.transport, session).await {
                Ok(token) => token.bearer,
                // The legacy HTTP adapter exposes no error-side close receipt.
                Err(_) => return PreparationResult::unconfirmed(),
            };
            if control.is_cancelled() {
                return PreparationResult::finished(Err(PreparationFailure::Cancelled));
            }
            let account = scope.account();
            if self.browser.seed_session(&account, &session.cookie_header).await.is_err() {
                // The current bridge has no error-side completion receipt.
                return PreparationResult::unconfirmed();
            }
            if control.is_cancelled() {
                return PreparationResult::finished(Err(PreparationFailure::Cancelled));
            }
            let token = match self.browser.castle_token(&account).await {
                Ok(token) => token,
                Err(_) => return PreparationResult::unconfirmed(),
            };
            if token.0.is_empty() || token.0.len() > 16 * 1024 {
                return PreparationResult::finished(Err(PreparationFailure::Rejected));
            }
            let castle = FreshCastleToken::from_browser(account, token);
            PreparationResult::finished(Ok(CredentialMaterial { bearer, castle }))
        }.boxed()
    }
}
