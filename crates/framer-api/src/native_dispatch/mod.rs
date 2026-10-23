//! Explicit owned adapter. Normal runtime never constructs this provider.
mod errors;
mod lifetime;
mod raw;
mod raw_stream;
mod composed_stream;
mod shared_lease;
pub mod account_lifecycle;
#[path = "complete_json/complete_json.rs"]
pub mod complete_json;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod native_tool_completion_tests;

#[cfg(test)]
mod raw_stream_tests;

#[cfg(test)]
mod raw_stream_start_tests;
#[cfg(test)]
mod start_cleanup_tests;
#[cfg(test)]
mod composed_stream_tests;
#[cfg(test)]
mod composed_budget_tests;
#[cfg(test)]
mod composed_refusal_tests;
#[cfg(test)]
mod composed_model_tests;
#[cfg(test)]
mod shared_lease_tests;

use crate::experimental_native::application::{AttemptAuthorization, AuthorizedAttempt,
    NativeInvocation, PreparedAttempt};
use crate::experimental_native::native_account::{AccountScope, NativeAccountClient};
use crate::HttpFramerClient;
use framer_core::account_lease::AccountLeaseCoordinator;
use framer_core::convert::{AssembledResponse, AssembledResponses, ClientDialect};
use framer_core::ids::{AccountId, ProjectId, TeamId};
use framer_core::native_dispatch::{self as port, Completed, DispatchError,
    FailureKind, InvocationProvider, InvocationRun, OwnedInvocation, Policy,
    Preparation, RawResult, Route};
use framer_core::port::ChainConfig;
use futures::{future::BoxFuture, FutureExt};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::Instant;

/// Explicit credit evidence, scoped by its containing immutable binding.
pub struct CreditEvidence { pub available: f64, pub valid_until: Instant }

pub struct Binding {
    pub scope: AccountScope,
    pub team: TeamId,
    pub project: ProjectId,
    pub authorization: Option<Arc<dyn AttemptAuthorization>>,
    pub paid_credit: Option<CreditEvidence>,
}

pub struct NativeDispatchProvider {
    api: Arc<HttpFramerClient>,
    native: NativeAccountClient,
    bindings: HashMap<AccountId, Binding>,
    policy: Policy,
    streaming: Option<port::StreamingPolicy>,
    account_leases: Option<AccountLeaseCoordinator>,
}

impl NativeDispatchProvider {
    pub fn new(api: Arc<HttpFramerClient>, native: NativeAccountClient,
        bindings: Vec<Binding>, policy: Policy) -> Result<Self, DispatchError> {
        policy.validate()?;
        let mut mapped = HashMap::new();
        for binding in bindings {
            if binding.team.0.is_empty() || binding.project.0.is_empty()
                || mapped.insert(binding.scope.account(), binding).is_some() {
                return Err(DispatchError::new(FailureKind::InvalidScope));
            }
        }
        Ok(Self { api, native, bindings: mapped, policy, streaming: None, account_leases: None })
    }

    pub fn with_streaming_policy(mut self, policy: port::StreamingPolicy)
        -> Result<Self, DispatchError> {
        policy.validate()?;
        self.streaming = Some(policy);
        Ok(self)
    }

    /// Attach the same coordinator used by legacy work and lifecycle changes.
    /// An occupied account fails prepare immediately because its caller already
    /// owns workspace capacity. No new coordinator or account binding is made.
    pub fn with_account_leases(mut self, coordinator: AccountLeaseCoordinator) -> Self {
        self.account_leases = Some(coordinator);
        self
    }
}

impl InvocationProvider for NativeDispatchProvider {
    fn policy(&self) -> Policy { self.policy.clone() }
    fn streaming_policy(&self) -> Option<port::StreamingPolicy> { self.streaming }
    fn contains_account(&self, account: AccountId) -> bool { self.bindings.contains_key(&account) }

    fn prepare(&self, request: Preparation, permit: OwnedSemaphorePermit)
        -> BoxFuture<'_, Result<Box<dyn OwnedInvocation>, DispatchError>> {
        self.prepare_owned(request, permit, None)
    }
    fn supports_pool_admission(&self) -> bool { true }
    fn prepare_in_pool(&self, request: Preparation, permit: OwnedSemaphorePermit,
        admission: port::PoolAdmission)
        -> BoxFuture<'_, Result<Box<dyn OwnedInvocation>, DispatchError>> {
        if admission.account() != request.identity.account {
            return async { Err(DispatchError::new(FailureKind::InvalidScope)) }.boxed();
        }
        self.prepare_owned(request, permit, Some(admission))
    }
}

impl NativeDispatchProvider {
    fn prepare_owned(&self, request: Preparation, permit: OwnedSemaphorePermit,
        pool: Option<port::PoolAdmission>)
        -> BoxFuture<'_, Result<Box<dyn OwnedInvocation>, DispatchError>> {
        async move {
            let binding = self.bindings.get(&request.identity.account)
                .ok_or_else(|| DispatchError::new(FailureKind::InvalidScope))?;
            if binding.scope.active_user() != &request.identity.user
                || binding.project != request.identity.project
                || binding.team != request.identity.team {
                return Err(DispatchError::new(FailureKind::InvalidScope));
            }
            if request.identity.chat_session.is_empty() || request.max_total_tokens == 0 {
                return Err(DispatchError::new(FailureKind::MissingContext));
            }
            if request.deadline <= Instant::now() {
                return Err(DispatchError::new(FailureKind::Deadline));
            }
            if binding.scope.closed().now_or_never().is_some() {
                return Err(DispatchError::new(FailureKind::ScopeClosed));
            }
            let authorization = binding.authorization.clone()
                .ok_or_else(|| DispatchError::new(FailureKind::MissingAuthorization))?;
            self.check_paid(binding, request.route)?;
            let account_lease = shared_lease::AccountLease::admit(self.account_leases.as_ref(),
                request.identity.account, &request.identity.user.0, pool)?;
            // Admission rechecks the coordinator binding. Retain prepared
            // ownership while checking scope and the original deadline again.
            if request.deadline <= Instant::now() {
                return Err(DispatchError::new(FailureKind::Deadline));
            }
            if binding.scope.closed().now_or_never().is_some() {
                return Err(DispatchError::new(FailureKind::ScopeClosed));
            }
            Ok(Box::new(NativeCall {
                api: self.api.clone(), native: self.native.clone(),
                scope: binding.scope.clone(), authorization,
                request, policy: self.policy.clone(), permit, account_lease,
            }) as Box<dyn OwnedInvocation>)
        }.boxed()
    }
}

impl NativeDispatchProvider {
    fn check_paid(&self, binding: &Binding, route: Route) -> Result<(), DispatchError> {
        if !matches!(route, Route::Raw(_)) { return Ok(()); }
        if !self.policy.allow_paid { return Err(DispatchError::new(FailureKind::PaidDisabled)); }
        let valid = binding.paid_credit.as_ref().is_some_and(|credit| {
            credit.available.is_finite() && credit.available >= self.policy.credit_floor
                && credit.valid_until > Instant::now()
        });
        if !valid { return Err(DispatchError::new(FailureKind::CreditFloor)); }
        Ok(())
    }
}

struct NativeCall {
    api: Arc<HttpFramerClient>, native: NativeAccountClient, scope: AccountScope,
    authorization: Arc<dyn AttemptAuthorization>, request: Preparation,
    policy: Policy, permit: OwnedSemaphorePermit,
    account_lease: shared_lease::AccountLease,
}

struct Parts {
    api: Arc<HttpFramerClient>, native: NativeAccountClient, scope: AccountScope,
    authorization: Arc<dyn AttemptAuthorization>, request: Preparation, policy: Policy,
}

impl Parts {
    fn invocation<'a>(&'a self, auth: &'a dyn AttemptAuthorization) -> NativeInvocation<'a> {
        NativeInvocation { client: &self.native, scope: &self.scope,
            authorization: Some(auth), deadline: self.request.deadline,
            missing_content_type: &self.policy.missing_content_type,
            max_body_bytes: self.policy.max_body_bytes }
    }
    fn config(&self) -> ChainConfig {
        ChainConfig { chunk_tokens: self.policy.chunk_tokens,
            max_chunks: self.policy.max_chunks,
            continue_prompt: self.policy.continuation.clone().unwrap_or_default(),
            max_total_tokens: Some(self.request.max_total_tokens),
            metered_fallback: false, on_429: None, cancelled: None }
    }
}

impl NativeCall {
    fn launch<T, F, Fut>(self: Box<Self>, operation: F) -> InvocationRun<T>
    where T: Send + 'static,
        F: FnOnce(Parts, Arc<lifetime::Registry>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<T, DispatchError>> + Send + 'static,
    {
        let Self { api, native, scope, authorization, request, policy, permit, account_lease } = *self;
        let deadline = request.deadline;
        let max_attempts = policy.max_chunks as usize;
        let parts = Parts { api, native: native.clone(), scope: scope.clone(), authorization, request, policy };
        lifetime::launch_with_lease(native, scope, permit, account_lease.into_running(), deadline, max_attempts,
            move |registry| operation(parts, registry))
    }
}

struct TrackedAuthorization {
    inner: Arc<dyn AttemptAuthorization>, registry: Arc<lifetime::Registry>,
}
impl TrackedAuthorization {
    fn error(&self, error: crate::experimental_native::application::ApplicationError) -> DispatchError {
        use crate::experimental_native::application::ApplicationError;
        use crate::experimental_native::native_account::ClientError;
        use crate::experimental_native::native_owner::AdmissionError;
        fn needs_account_cleanup(error: &ApplicationError) -> bool {
            match error {
                ApplicationError::Start(ClientError::Draining | ClientError::Attachment
                    | ClientError::Admission(AdmissionError::SpawnFailed)) => true,
                ApplicationError::Chain { error, .. } => needs_account_cleanup(error),
                _ => false,
            }
        }
        // Keep this evidence before mapping away the native start-error type.
        if needs_account_cleanup(&error) { self.registry.require_account_cleanup(); }
        errors::error(error)
    }
}
impl AttemptAuthorization for TrackedAuthorization {
    fn observe_finish(&self, metadata: &crate::experimental_native::application::AttemptMetadata) {
        self.registry.completed(errors::metadata(metadata.clone()));
        self.inner.observe_finish(metadata);
    }
    fn authorize<'a>(&'a self, attempt: PreparedAttempt<'a>)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        async move {
            let observer = |cleanup, cancellation| self.registry.preparing(cleanup, cancellation);
            self.inner.authorize_observed(attempt, &observer).await
        }.boxed()
    }
    fn observe_start(&self,
        cleanup: crate::experimental_native::downstream_owner::Cleanup,
        cancellation: crate::experimental_native::native_owner::Cancellation) {
        self.registry.started(cleanup.clone(), cancellation.clone());
        self.inner.observe_start(cleanup, cancellation);
    }
}

fn validate_payload(payload: &Value) -> Result<(), DispatchError> {
    if !payload.is_object() || payload.get("stream").is_some_and(|v| !v.is_boolean()) {
        return Err(DispatchError::new(FailureKind::InvalidRequest));
    }
    if payload["stream"].as_bool() == Some(true) {
        return Err(DispatchError::new(FailureKind::UnsupportedStreaming));
    }
    Ok(())
}

impl OwnedInvocation for NativeCall {
    fn composed_stream_for_client(self: Box<Self>, payload: Value,
        options: port::ComposedStreamOptions, client_model: String) -> port::ComposedStreamRun {
        let mut run = composed_stream::launch_for_client(self, payload, options, client_model);
        run.response.cleanup = observed_stream_cleanup(run.response.cleanup);
        run
    }
    fn composed_stream(self: Box<Self>, payload: Value, options: port::ComposedStreamOptions)
        -> port::ComposedStreamRun {
        let mut run = composed_stream::launch(self, payload, options);
        run.response.cleanup = observed_stream_cleanup(run.response.cleanup);
        run
    }
    fn raw(self: Box<Self>, payload: Value) -> InvocationRun<RawResult> {
        self.launch(move |p, registry| async move {
            validate_payload(&payload)?;
            let Route::Raw(dialect) = p.request.route else {
                return Err(DispatchError::new(FailureKind::InvalidRequest));
            };
            let auth = TrackedAuthorization { inner: p.authorization.clone(), registry };
            raw::collect(&p, &auth, dialect, payload).await
        })
    }

    fn raw_stream(self: Box<Self>, payload: Value, limits: port::RawStreamLimits)
        -> port::RawStreamRun {
        let mut run = raw_stream::launch(self, payload, limits);
        run.response.cleanup = observed_stream_cleanup(run.response.cleanup);
        run
    }

    fn messages(self: Box<Self>, payload: Value) -> InvocationRun<Completed<AssembledResponse>> {
        self.launch(move |p, registry| async move {
            validate_payload(&payload)?;
            if p.request.route != Route::Messages {
                return Err(DispatchError::new(FailureKind::InvalidRequest));
            }
            let auth = TrackedAuthorization { inner: p.authorization.clone(), registry };
            let invocation = p.invocation(&auth);
            let result = p.api.freechat_long_native(&invocation, &p.request.identity.project,
                payload, &p.config(), &p.request.identity.chat_session, None)
                .await.map_err(|error| auth.error(error))?;
            Ok(Completed { value: result.response,
                attempts: result.attempts.into_iter().map(errors::metadata).collect() })
        })
    }

    fn responses(self: Box<Self>, payload: Value) -> InvocationRun<Completed<AssembledResponses>> {
        self.launch(move |p, registry| async move {
            validate_payload(&payload)?;
            if p.request.route != Route::Responses {
                return Err(DispatchError::new(FailureKind::InvalidRequest));
            }
            if ["store", "background", "previous_response_id"].iter()
                .any(|key| payload.get(*key).is_some())
                || payload["model"].as_str().is_some_and(|model| model.ends_with(":batch")) {
                return Err(DispatchError::new(FailureKind::InvalidRequest));
            }
            reject_namespace_repair(&payload)?;
            let auth = TrackedAuthorization { inner: p.authorization.clone(), registry };
            let invocation = p.invocation(&auth);
            let result = p.api.native_long_native(&invocation, &p.request.identity.project,
                payload, &p.config(), &p.request.identity.chat_session, None)
                .await.map_err(|error| auth.error(error))?;
            Ok(Completed { value: result.response,
                attempts: result.attempts.into_iter().map(errors::metadata).collect() })
        })
    }
}

fn observed_stream_cleanup(cleanup: port::Cleanup) -> port::Cleanup {
    async move {
        let result = cleanup.await;
        crate::experimental_native::native_http::send_evidence::settled(result.is_ok());
        result
    }.boxed().shared()
}

// The legacy builder can backfill namespaces. Native admission rejects such
// history instead, so the accepted builder cannot silently alter it.
fn reject_namespace_repair(payload: &Value) -> Result<(), DispatchError> {
    let mut declared = std::collections::HashSet::new();
    for tool in payload["tools"].as_array().into_iter().flatten() {
        if tool["type"].as_str() == Some("namespace") {
            if tool["name"].is_string() {
                for inner in tool["tools"].as_array().into_iter().flatten() {
                    if let Some(name) = inner["name"].as_str() { declared.insert(name); }
                }
            }
        } else if let Some(name) = tool["name"].as_str() {
            if tool["namespace"].is_string() { declared.insert(name); }
            else if name.contains('.') { declared.insert(name.rsplit('.').next().unwrap_or(name)); }
        }
    }
    for item in payload["input"].as_array().into_iter().flatten() {
        if matches!(item["type"].as_str(), Some("function_call") | Some("custom_tool_call"))
            && item.get("namespace").is_none()
            && item["name"].as_str().is_some_and(|name| declared.contains(name)) {
            return Err(DispatchError::new(FailureKind::InvalidRequest));
        }
    }
    Ok(())
}
