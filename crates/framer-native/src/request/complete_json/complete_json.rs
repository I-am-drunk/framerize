//! Bounded request contract. JSON stays one-attempt; streams may opt into continuation.
use crate::request::invocation::AttemptAuthorization;
use crate::request::invocation::AttemptContext;
use crate::request::invocation::AttemptMetadata;
use crate::request::invocation::AuthorizedAttempt;
use crate::request::invocation::PreparationObserver;
use crate::request::invocation::PreparedAttempt;
use crate::transport::downstream as downstream_owner;
use crate::account::owner as native_owner;
use crate::account::client::AccountScope;
use framer_api::proxy::Dialect;
use framer_core::convert::AssembledResponse;
use framer_core::convert::AssembledResponses;
use crate::request::types as port;
use futures::future::BoxFuture;
use futures::FutureExt;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::Instant;
use super::continuation::Continuation;

#[path = "supplied_source.rs"]
mod supplied_source;

#[path = "refusal_evidence.rs"]
mod refusal_evidence;
pub use framer_api::classify::Refusal;
pub use framer_api::classify::RefusalBasis;
pub use framer_api::classify::RefusalObservation;

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
    /// Only the original, complete terminal inference refusal is classified.
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

impl CompletedJson {
    pub(crate) fn complete(&self) -> bool {
        match self {
            Self::Responses(value) => complete_responses(value, 1),
            Self::Messages(value) => complete_messages(value, 1),
        }
    }
}

fn complete_responses(value: &AssembledResponses, max_attempts: usize) -> bool {
    !value.id.is_empty() && value.status == "completed"
        && value.incomplete_details.as_ref().map_or(true, Value::is_null)
        && (1..=max_attempts).contains(&(value.chunks as usize))
        && value.metered_chunks == 0 && !value.output.is_empty()
}

fn complete_messages(value: &AssembledResponse, max_attempts: usize) -> bool {
    !value.id.is_empty()
        && matches!(value.stop_reason.as_str(), "end_turn" | "stop_sequence" | "tool_use")
        && (1..=max_attempts).contains(&(value.chunks as usize))
        && value.metered_chunks == 0 && !value.content.is_empty()
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

pub(super) fn content_type(value: Option<&str>) -> ContentType {
    let Some(value) = value else { return ContentType::Missing; };
    let media = value.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    if media == "text/event-stream" { ContentType::EventStream }
    else if media == "application/json"
        || (media.starts_with("application/") && media.ends_with("+json")) { ContentType::Json }
    else { ContentType::Other }
}

fn responses(value: port::Completed<AssembledResponses>, max_attempts: usize) -> Facts {
    Facts {
        complete: complete_responses(&value.value, max_attempts),
        input_tokens: value.value.usage["input_tokens"].as_u64(),
        output_tokens: value.value.usage["output_tokens"].as_u64(),
        output_items: value.value.output.len(), attempts: value.attempts, failure: None,
        refusal: None,
        completed: Some(CompletedJson::Responses(value.value)),
    }
}

fn messages(value: port::Completed<AssembledResponse>, max_attempts: usize) -> Facts {
    Facts {
        complete: complete_messages(&value.value, max_attempts),
        input_tokens: value.value.usage["input_tokens"].as_u64(),
        output_tokens: value.value.usage["output_tokens"].as_u64(),
        output_items: value.value.content.len(), attempts: value.attempts, failure: None,
        refusal: None,
        completed: Some(CompletedJson::Messages(value.value)),
    }
}

fn failure(value: port::DispatchError, max_attempts: usize) -> Facts {
    let refusal = refusal_evidence::observe_with_limit(&value, max_attempts);
    let mut attempts = value.attempts;
    if let Some(upstream) = value.upstream {
        if !attempts.last().is_some_and(|last| refusal_evidence::same_metadata(last, &upstream.metadata)) {
            attempts.push(upstream.metadata);
        }
        // Only the bounded classification survives. Drop the private body.
    }
    Facts { complete: false, attempts, input_tokens: None, output_tokens: None,
        output_items: 0, failure: Some(value.kind), refusal, completed: None }
}

fn evidence(facts: Facts, contract: &Contract) -> Evidence {
    let metadata = if contract.continuation.is_some() && facts.failure.is_some() {
        facts.attempts.last()
    } else { facts.attempts.first() };
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
    inspect: impl FnOnce(T) -> Facts + Send + 'static) -> ProbeRun {
    let port::InvocationRun { result, cleanup, cancellation, .. } = run;
    let completion = cleanup.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let result = async move {
        let mut facts = match result.await {
            Ok(value) => inspect(value), Err(error) => failure(error, contract.max_attempts()),
        };
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
            && report.status == Some(200) && (1..=contract.max_attempts()).contains(&report.attempts)
            && report.content_type == ContentType::Json
            && report.authorization_calls == report.attempts && report.transport_starts == report.attempts
            && report.transport_finishes == report.attempts && report.failure.is_none()
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
    events: Option<super::ChunkSender>,
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
    continuation: Option<Continuation>,
}

impl Contract {
    fn max_attempts(&self) -> usize {
        self.continuation.as_ref().map_or(1, Continuation::max_attempts)
    }
    fn context_at(&self, index: usize) -> Option<&AttemptContext> {
        if index == 0 { Some(&self.context) }
        else { self.continuation.as_ref()?.contexts.get(index - 1) }
    }
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
            let input = framer_api::protocol::responses::prepare_input(&spec.payload);
            Ok(framer_api::protocol::responses::prepare_payload(&spec.payload, &input, spec.token_limit))
        }
        FreeRoute::Messages => {
            if spec.route.dialect().path() != "/ai/v3/messages/"
                || !spec.payload["messages"].is_array() {
                return Err(invalid());
            }
            Ok(framer_api::protocol::messages::prepare_payload(&spec.payload, &spec.payload["messages"],
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
        "request authorization contract rejected".into()
    }
    fn check(&self, attempt: &PreparedAttempt<'_>) -> Result<(), String> {
        let c = &self.contract;
        let index = attempt.step_index as usize;
        let payload_matches = if index == 0 { attempt.payload == &c.expected } else {
            c.continuation.as_ref().is_some_and(|options| bounded_json(attempt.payload)
                && options.matches_payload(&c.expected, attempt.payload,
                    c.path == "/ai/v3/responses/", index))
        };
        if c.rejected.load(Ordering::Acquire)
            || index >= c.max_attempts()
            || c.calls.compare_exchange(index, index + 1, Ordering::AcqRel, Ordering::Acquire).is_err()
            || !c.scope.same_generation(attempt.scope)
            || attempt.project != &c.identity.project
            || attempt.chat_session != c.identity.chat_session
            || attempt.deadline != c.deadline
            || attempt.dialect.path() != c.path || !payload_matches {
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
            let index = attempt.step_index as usize;
            self.check(&attempt)?;
            let authorized = self.inner.authorize_observed(attempt, observer).await?;
            let context = &authorized.context;
            let expected = self.contract.context_at(index).expect("checked attempt index");
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
        if self.contract.continuation.is_some() && (metadata.status != 200
            || content_type(metadata.original_content_type.as_deref()) != ContentType::Json) {
            self.contract.rejected.store(true, Ordering::Release);
        }
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
    pub(crate) fn new_with_continuation(scope: AccountScope, spec: Specification,
        continuation: Option<Continuation>) -> Result<Self, port::DispatchError> {
        if let Some(options) = &continuation { options.validate(&spec)?; }
        let mut plan = Self::new(scope, spec)?;
        Arc::get_mut(&mut plan.contract).expect("new request contract").continuation = continuation;
        Ok(plan)
    }

    pub(crate) fn with_events(mut self, events: Option<super::ChunkSender>) -> Self {
        self.events = events;
        self
    }

    /// Execute once while retaining the caller's pool and operation owners.
    pub async fn launch_in_pool(self,
        native: crate::account::client::NativeAccountClient,
        authorization: Arc<dyn AttemptAuthorization>, permit: OwnedSemaphorePermit,
        admission: port::PoolAdmission) -> Result<ProbeRun, port::DispatchError> {
        if admission.account() != self.contract.identity.account {
            return Err(port::DispatchError::new(port::FailureKind::InvalidScope));
        }
        if self.contract.deadline <= Instant::now() {
            return Err(port::DispatchError::new(port::FailureKind::Deadline));
        }
        if self.contract.scope.closed().now_or_never().is_some() {
            return Err(port::DispatchError::new(port::FailureKind::ScopeClosed));
        }
        let Self { payload, route, token_limit, contract, events } = self;
        let max_attempts = contract.max_attempts();
        let config = framer_core::port::ChainConfig {
            chunk_tokens: token_limit, max_chunks: max_attempts as u32,
            continue_prompt: contract.continuation.as_ref().map_or_else(String::new, |v| v.prompt.clone()),
            max_total_tokens: Some(contract.continuation.as_ref().map_or(token_limit, |v| v.max_output_tokens)),
            metered_fallback: false, on_429: None, cancelled: None,
        };
        let run = super::execution::execute(super::execution::Execution {
            client: native, scope: contract.scope.clone(), identity: contract.identity.clone(),
            authorization: Arc::new(CheckedAuthorization { inner: authorization, contract: contract.clone() }),
            payload, route, config, deadline: contract.deadline, permit, admission, events,
        });
        Ok(finish_run(run, contract, move |completed| match completed.value {
            CompletedJson::Responses(value) => responses(port::Completed { value, attempts: completed.attempts }, max_attempts),
            CompletedJson::Messages(value) => messages(port::Completed { value, attempts: completed.attempts }, max_attempts),
        }))
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
        if matches!(spec.route, FreeRoute::Responses) { super::validation::reject_namespace_repair(&spec.payload)?; }
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
        let contract = Arc::new(Contract {
            scope, identity: spec.identity, path: spec.route.dialect().path(),
            client_model: model.to_owned(),
            wire_model: expected["model"].as_str().ok_or_else(invalid)?.to_owned(),
            expected, context: spec.context,
            deadline: spec.deadline,
            calls: AtomicUsize::new(0), starts: AtomicUsize::new(0),
            finishes: AtomicUsize::new(0), rejected: AtomicBool::new(false), continuation: None,
        });
        Ok(Self { payload: spec.payload, route: spec.route,
            token_limit: spec.token_limit, contract, events: None })
    }
}
