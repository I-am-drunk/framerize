//! Source-only one-shot acceptance adapter. No CLI or credential acquisition.
//! Include inside framer-api only after review; this is not a live opt-in.
use crate::experimental_native::application::{AttemptAuthorization, AttemptContext, AttemptMetadata,
    AuthorizedAttempt, PreparationObserver, PreparedAttempt};
use crate::experimental_native::{downstream_owner, native_owner};
use crate::experimental_native::native_account::AccountScope;
use crate::native_dispatch::Binding;
#[cfg(test)]
use crate::native_dispatch::{CreditEvidence, NativeDispatchProvider};
use crate::proxy::Dialect;
use framer_core::convert::{AssembledResponse, AssembledResponses};
use framer_core::native_dispatch as port;
use futures::{future::BoxFuture, FutureExt};
use serde_json::Value;
use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::Duration;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::Instant;

#[path = "supplied_source.rs"]
mod supplied_source;

#[path = "refusal_evidence.rs"]
mod refusal_evidence;
pub use crate::classify::{Refusal, RefusalBasis, RefusalObservation};

#[cfg(test)]
#[path = "complete_json_tests.rs"]
mod tests;

const BYTE_LIMIT: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentType { Json, EventStream, Other, Missing }

/// Safe evidence only. No response text, identities, credentials or raw headers.
#[derive(Debug)]
pub struct Evidence {
    pub accepted: bool,
    pub path: &'static str,
    pub client_model: String,
    pub wire_model: String,
    pub status: Option<u16>,
    pub content_type: ContentType,
    pub usage_header_present: bool,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub output_items: usize,
    pub attempts: usize,
    pub authorization_calls: usize,
    pub transport_starts: usize,
    pub transport_finishes: usize,
    pub complete_json: bool,
    pub cleanup_confirmed: bool,
    pub failure: Option<port::FailureKind>,
    /// Only the original, complete, single inference refusal is classified.
    /// None is unknown or not applicable, never evidence of account health.
    pub refusal: Option<RefusalObservation>,
}

/// Keep cleanup independently if result is dropped or a caller deadline wins.
#[must_use = "consume or cancel the result and retain independent cleanup"]
pub struct ProbeRun {
    pub result: BoxFuture<'static, Evidence>,
    pub cleanup: port::Cleanup,
    pub cancellation: Arc<dyn port::Cancellation>,
    /// The same completed response, released only after acceptance and cleanup.
    /// Poll result before or alongside this receiver. No second request is made.
    pub completed: BoxFuture<'static, Option<CompletedJson>>,
}

pub enum CompletedJson {
    Responses(AssembledResponses),
    Messages(AssembledResponse),
}

struct Facts {
    complete: bool,
    attempts: Vec<port::AttemptMetadata>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    output_items: usize,
    failure: Option<port::FailureKind>,
    refusal: Option<RefusalObservation>,
    completed: Option<CompletedJson>,
}

fn content_type(value: Option<&str>) -> ContentType {
    let Some(value) = value else { return ContentType::Missing; };
    let media = value.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    if media == "text/event-stream" { ContentType::EventStream }
    else if media == "application/json"
        || (media.starts_with("application/") && media.ends_with("+json")) { ContentType::Json }
    else { ContentType::Other }
}

fn responses(value: port::Completed<AssembledResponses>) -> Facts {
    Facts {
        complete: !value.value.id.is_empty() && value.value.status == "completed"
            && value.value.incomplete_details.as_ref().map_or(true, Value::is_null)
            && value.value.chunks == 1 && value.value.metered_chunks == 0
            && !value.value.output.is_empty(),
        input_tokens: value.value.usage["input_tokens"].as_u64(),
        output_tokens: value.value.usage["output_tokens"].as_u64(),
        output_items: value.value.output.len(), attempts: value.attempts, failure: None,
        refusal: None,
        completed: Some(CompletedJson::Responses(value.value)),
    }
}

fn messages(value: port::Completed<AssembledResponse>) -> Facts {
    Facts {
        complete: !value.value.id.is_empty()
            && matches!(value.value.stop_reason.as_str(), "end_turn" | "stop_sequence" | "tool_use")
            && value.value.chunks == 1 && value.value.metered_chunks == 0
            && !value.value.content.is_empty(),
        input_tokens: value.value.usage["input_tokens"].as_u64(),
        output_tokens: value.value.usage["output_tokens"].as_u64(),
        output_items: value.value.content.len(), attempts: value.attempts, failure: None,
        refusal: None,
        completed: Some(CompletedJson::Messages(value.value)),
    }
}

fn failure(value: port::DispatchError) -> Facts {
    let refusal = refusal_evidence::observe(&value);
    let mut attempts = value.attempts;
    if let Some(upstream) = value.upstream {
        if attempts.is_empty() { attempts.push(upstream.metadata); }
        // Only the bounded classification survives. Drop the private body.
    }
    Facts { complete: false, attempts, input_tokens: None, output_tokens: None,
        output_items: 0, failure: Some(value.kind), refusal, completed: None }
}

fn evidence(facts: Facts, contract: &Contract) -> Evidence {
    let metadata = facts.attempts.first();
    Evidence {
        accepted: false, path: contract.path, client_model: contract.client_model.clone(),
        wire_model: contract.wire_model.clone(), status: metadata.map(|m| m.status),
        content_type: content_type(metadata.and_then(|m| m.original_content_type.as_deref())),
        usage_header_present: metadata.is_some_and(|m| m.usage_header.is_some()),
        input_tokens: facts.input_tokens, output_tokens: facts.output_tokens,
        output_items: facts.output_items, attempts: facts.attempts.len(),
        authorization_calls: 0, transport_starts: 0, transport_finishes: 0,
        complete_json: facts.complete, cleanup_confirmed: false, failure: facts.failure,
        refusal: facts.refusal,
    }
}

fn finish_run<T: Send + 'static>(run: port::InvocationRun<T>, contract: Arc<Contract>,
    inspect: fn(T) -> Facts) -> ProbeRun {
    let port::InvocationRun { result, cleanup, cancellation, .. } = run;
    let completion = cleanup.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let result = async move {
        let mut facts = match result.await { Ok(value) => inspect(value), Err(error) => failure(error) };
        let completed = facts.completed.take();
        let mut report = evidence(facts, &contract);
        report.cleanup_confirmed = completion.await.is_ok();
        if !report.cleanup_confirmed && report.failure.is_none() {
            report.failure = Some(port::FailureKind::Cleanup);
        }
        report.authorization_calls = contract.calls.load(Ordering::Acquire);
        report.transport_starts = contract.starts.load(Ordering::Acquire);
        report.transport_finishes = contract.finishes.load(Ordering::Acquire);
        report.accepted = report.complete_json && report.cleanup_confirmed
            && report.status == Some(200) && report.attempts == 1
            && report.content_type == ContentType::Json
            && report.authorization_calls == 1 && report.transport_starts == 1
            && report.transport_finishes == 1 && report.failure.is_none()
            && !contract.rejected.load(Ordering::Acquire);
        let _ = sender.send(if report.accepted { completed } else { None });
        report
    }.boxed();
    let completed = async move { receiver.await.unwrap_or(None) }.boxed();
    ProbeRun { result, cleanup, cancellation, completed }
}

#[derive(Clone, Copy, Debug)]
pub enum FreeRoute { Responses, Messages }
impl FreeRoute {
    fn dialect(self) -> Dialect {
        match self { Self::Responses => Dialect::OpenRouterResponses,
            Self::Messages => Dialect::OpenRouterMessages }
    }
    fn route(self) -> port::Route {
        match self { Self::Responses => port::Route::Responses,
            Self::Messages => port::Route::Messages }
    }
}

/// These values must come from the actual authorized feature and conversation.
pub struct Specification {
    pub identity: port::Identity,
    pub route: FreeRoute,
    pub payload: Value,
    pub context: AttemptContext,
    pub deadline: Instant,
    pub token_limit: u32,
}

pub struct Plan {
    payload: Value,
    route: FreeRoute,
    token_limit: u32,
    contract: Arc<Contract>,
    policy: port::Policy,
}

struct Contract {
    scope: AccountScope,
    identity: port::Identity,
    expected: Value,
    path: &'static str,
    client_model: String,
    wire_model: String,
    context: AttemptContext,
    deadline: Instant,
    calls: AtomicUsize,
    starts: AtomicUsize,
    finishes: AtomicUsize,
    rejected: AtomicBool,
}

fn invalid() -> port::DispatchError {
    port::DispatchError::new(port::FailureKind::InvalidRequest)
}

fn expected_payload(spec: &Specification) -> Result<Value, port::DispatchError> {
    match spec.route {
        FreeRoute::Responses => {
            if spec.route.dialect().path() != "/ai/v3/responses/"
                || !(spec.payload["input"].is_string() || spec.payload["input"].is_array())
                || ["store", "background", "previous_response_id"].iter()
                    .any(|key| spec.payload.get(*key).is_some())
                || spec.payload["model"].as_str().is_some_and(|m| m.ends_with(":batch")) {
                return Err(invalid());
            }
            let input = crate::nativechain::prepare_input(&spec.payload);
            Ok(crate::nativechain::prepare_payload(&spec.payload, &input, spec.token_limit))
        }
        FreeRoute::Messages => {
            if spec.route.dialect().path() != "/ai/v3/messages/"
                || !spec.payload["messages"].is_array() {
                return Err(invalid());
            }
            Ok(crate::freechat::prepare_payload(&spec.payload, &spec.payload["messages"],
                spec.token_limit, Dialect::OpenRouterMessages))
        }
    }
}

struct CheckedAuthorization {
    inner: Arc<dyn AttemptAuthorization>,
    contract: Arc<Contract>,
}
impl CheckedAuthorization {
    fn reject(&self) -> String {
        self.contract.rejected.store(true, Ordering::Release);
        "one-shot authorization contract rejected".into()
    }
    fn check(&self, attempt: &PreparedAttempt<'_>) -> Result<(), String> {
        let c = &self.contract;
        if c.rejected.load(Ordering::Acquire)
            || c.calls.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_err()
            || !c.scope.same_generation(attempt.scope)
            || attempt.project != &c.identity.project
            || attempt.chat_session != c.identity.chat_session
            || attempt.deadline != c.deadline
            || attempt.step_index != 0 || attempt.dialect.path() != c.path
            || attempt.payload != &c.expected {
            return Err(self.reject());
        }
        Ok(())
    }
}

impl AttemptAuthorization for CheckedAuthorization {
    fn authorize<'a>(&'a self, _: PreparedAttempt<'a>)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        // A bypass of the observed guardian must fail before issuing credentials.
        futures::future::ready(Err(self.reject())).boxed()
    }
    fn authorize_observed<'a>(&'a self, attempt: PreparedAttempt<'a>,
        observer: &'a dyn PreparationObserver)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        async move {
            self.check(&attempt)?;
            let authorized = self.inner.authorize_observed(attempt, observer).await?;
            let context = &authorized.context;
            let expected = &self.contract.context;
            if context.feature_category != expected.feature_category
                || context.feature_name != expected.feature_name
                || context.step != expected.step
                || context.turn_id != expected.turn_id
                || context.user_prompt != expected.user_prompt {
                return Err(self.reject());
            }
            Ok(authorized)
        }.boxed()
    }
    fn observe_start(&self, cleanup: downstream_owner::Cleanup,
        cancellation: native_owner::Cancellation) {
        self.contract.starts.fetch_add(1, Ordering::AcqRel);
        self.inner.observe_start(cleanup, cancellation);
    }
    fn observe_finish(&self, metadata: &AttemptMetadata) {
        self.contract.finishes.fetch_add(1, Ordering::AcqRel);
        self.inner.observe_finish(metadata);
    }
}

fn bounded_json(value: &Value) -> bool {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > BYTE_LIMIT.saturating_sub(self.0) {
                return Err(std::io::Error::other("probe payload exceeds limit"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    serde_json::to_writer(Counter(0), value).is_ok()
}

impl Plan {
    /// Exact pre-token envelope for an explicit browser/native comparison.
    /// Uses the same serializer as native dispatch, including session binding.
    pub fn comparison_body(&self) -> Result<Value, port::DispatchError> {
        let c = &self.contract;
        let (path, body) = crate::experimental_native::native_account::RequestContext {
            dialect: self.route.dialect(), project: c.identity.project.clone(),
            feature_category: c.context.feature_category.clone(),
            feature_name: c.context.feature_name.clone(), step: c.context.step.clone(),
            chat_session: c.identity.chat_session.clone(), step_index: 0,
            turn_id: c.context.turn_id.clone(), user_prompt: c.context.user_prompt.clone(),
            payload: c.expected.clone(),
        }.prepare().map_err(|_| invalid())?;
        if path != c.path || !bounded_json(&body) { return Err(invalid()); }
        Ok(body)
    }

    /// The factory must attach the root-owned shared legacy/native lease and
    /// mutation fencing. It receives the checked binding, never raw credentials.
    /// A bare NativeDispatchProvider is suitable only for isolated fixtures.
    pub async fn launch<F>(self, authorization: Arc<dyn AttemptAuthorization>,
        permit: OwnedSemaphorePermit, factory: F) -> Result<ProbeRun, port::DispatchError>
    where F: FnOnce(Binding, port::Policy)
        -> Result<Arc<dyn port::InvocationProvider>, port::DispatchError> {
        self.launch_owned(authorization, permit, None, factory).await
    }

    /// Transfer the live application's existing resident and operation owners.
    pub async fn launch_in_pool<F>(self, authorization: Arc<dyn AttemptAuthorization>,
        permit: OwnedSemaphorePermit, admission: port::PoolAdmission, factory: F)
        -> Result<ProbeRun, port::DispatchError>
    where F: FnOnce(Binding, port::Policy)
        -> Result<Arc<dyn port::InvocationProvider>, port::DispatchError> {
        self.launch_owned(authorization, permit, Some(admission), factory).await
    }

    async fn launch_owned<F>(self, authorization: Arc<dyn AttemptAuthorization>,
        permit: OwnedSemaphorePermit, admission: Option<port::PoolAdmission>, factory: F)
        -> Result<ProbeRun, port::DispatchError>
    where F: FnOnce(Binding, port::Policy)
        -> Result<Arc<dyn port::InvocationProvider>, port::DispatchError> {
        if admission.as_ref().is_some_and(|a| a.account() != self.contract.identity.account) {
            return Err(port::DispatchError::new(port::FailureKind::InvalidScope));
        }
        let Self { payload, route, token_limit, contract, policy } = self;
        let binding = Binding { scope: contract.scope.clone(),
            team: contract.identity.team.clone(), project: contract.identity.project.clone(),
            authorization: Some(Arc::new(CheckedAuthorization {
                inner: authorization, contract: contract.clone(),
            })), paid_credit: None };
        let provider = factory(binding, policy.clone())?;
        let actual = provider.policy();
        if actual.max_chunks != 1 || actual.allow_paid || actual.continuation.is_some()
            || actual.chunk_tokens != policy.chunk_tokens
            || actual.max_body_bytes != policy.max_body_bytes
            || actual.timeout != policy.timeout
            || actual.missing_content_type != policy.missing_content_type
            || actual.credit_floor != 0.0
            || !provider.contains_account(contract.identity.account)
            || (admission.is_some() && !provider.supports_pool_admission()) {
            return Err(invalid());
        }
        let preparation = port::Preparation { identity: contract.identity.clone(),
            route: route.route(), deadline: contract.deadline,
            max_total_tokens: token_limit };
        let preparing = match admission {
            Some(admission) => provider.prepare_in_pool(preparation, permit, admission),
            None => provider.prepare(preparation, permit),
        };
        let call = tokio::time::timeout_at(contract.deadline, preparing).await
            .map_err(|_| port::DispatchError::new(port::FailureKind::Deadline))??;
        Ok(match route {
            FreeRoute::Responses => finish_run(call.responses(payload), contract, responses),
            FreeRoute::Messages => finish_run(call.messages(payload), contract, messages),
        })
    }

    /// Validate the complete request before account admission or private I/O.
    pub fn validate(spec: &Specification) -> Result<(), port::DispatchError> {
        Self::checked_payload(spec).map(|_| ())
    }

    fn checked_payload(spec: &Specification) -> Result<Value, port::DispatchError> {
        let remaining = spec.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || remaining > Duration::from_secs(60)
            || !(16..=512).contains(&spec.token_limit)
            || [&spec.identity.chat_session, &spec.identity.project.0,
                &spec.identity.team.0, &spec.identity.user.0]
                .iter().any(|value| value.is_empty() || value.len() > 4096
                    || value.chars().any(char::is_control))
            || !spec.payload.is_object() || spec.payload["stream"] != false
            || !bounded_json(&spec.payload) {
            return Err(invalid());
        }
        let model = spec.payload["model"].as_str().ok_or_else(invalid)?;
        if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control)
            || [&spec.context.feature_category, &spec.context.feature_name, &spec.context.step]
                .iter().any(|v| v.is_empty() || v.len() > 256 || v.chars().any(char::is_control))
            || spec.context.turn_id.is_empty() || spec.context.turn_id.len() > 4096
            || spec.context.turn_id.chars().any(char::is_control)
            || spec.context.user_prompt.len() > BYTE_LIMIT {
            return Err(invalid());
        }
        let expected = expected_payload(spec)?;
        if !bounded_json(&expected) { return Err(invalid()); }
        Ok(expected)
    }

    pub fn new(scope: AccountScope, spec: Specification) -> Result<Self, port::DispatchError> {
        if scope.account() != spec.identity.account
            || scope.active_user() != &spec.identity.user
            || scope.closed().now_or_never().is_some() {
            return Err(invalid());
        }
        let expected = Self::checked_payload(&spec)?;
        let model = spec.payload["model"].as_str().ok_or_else(invalid)?;
        let remaining = spec.deadline.saturating_duration_since(Instant::now());
        let contract = Arc::new(Contract {
            scope, identity: spec.identity, path: spec.route.dialect().path(),
            client_model: model.to_owned(),
            wire_model: expected["model"].as_str().ok_or_else(invalid)?.to_owned(),
            expected, context: spec.context,
            deadline: spec.deadline,
            calls: AtomicUsize::new(0), starts: AtomicUsize::new(0),
            finishes: AtomicUsize::new(0), rejected: AtomicBool::new(false),
        });
        let policy = port::Policy { timeout: remaining, max_body_bytes: BYTE_LIMIT,
            missing_content_type: "application/json".into(), chunk_tokens: spec.token_limit,
            max_chunks: 1, continuation: None, allow_paid: false, credit_floor: 0.0 };
        Ok(Self { payload: spec.payload, route: spec.route,
            token_limit: spec.token_limit, contract, policy })
    }
}
