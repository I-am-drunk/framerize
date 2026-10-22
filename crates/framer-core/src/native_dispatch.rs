//! Optional owned inference port. No credentials or provider-specific policy.
use crate::convert::{AssembledResponse, AssembledResponses, ClientDialect};
use crate::ids::{AccountId, FramerUserId, ProjectId, TeamId};
use futures::future::{BoxFuture, Shared};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::Instant;

#[path = "native_dispatch_admission.rs"]
mod admission;
pub use admission::PoolAdmission;

#[path = "native_dispatch_stream.rs"]
mod stream;
#[path = "native_dispatch_composed.rs"]
mod composed;
pub use composed::{ComposedAttemptMetadata, ComposedStreamCompletion,
    ComposedStreamFinished, ComposedStreamHead, ComposedStreamLimits,
    ComposedStreamOptions, ComposedStreamRun};
#[path = "native_stream_state.rs"]
pub mod stream_state;
#[path = "native_stream_frames.rs"]
pub mod stream_frames;
pub use stream::{RawStreamBody, RawStreamCompletion, RawStreamFinished,
    RawStreamHead, RawStreamHeaders, RawStreamLimits, RawStreamRun};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route { Raw(ClientDialect), Messages, Responses }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub account: AccountId,
    pub user: FramerUserId,
    pub team: TeamId,
    pub project: ProjectId,
    pub chat_session: String,
}

/// Explicit caller policy. There is deliberately no invented continuation.
#[derive(Clone)]
pub struct Policy {
    pub timeout: Duration,
    pub max_body_bytes: usize,
    pub missing_content_type: String,
    pub chunk_tokens: u32,
    pub max_chunks: u32,
    pub continuation: Option<String>,
    pub allow_paid: bool,
    pub credit_floor: f64,
}

impl Policy {
    pub fn validate(&self) -> Result<(), DispatchError> {
        if self.timeout.is_zero() || self.max_body_bytes == 0
            || self.missing_content_type.is_empty()
            || self.missing_content_type.contains(['\r', '\n'])
            || self.max_body_bytes > 64 * 1024 * 1024 || self.chunk_tokens == 0
            || !(1..=64).contains(&self.max_chunks)
            || !self.credit_floor.is_finite() || self.credit_floor < 0.0 {
            return Err(DispatchError::new(FailureKind::InvalidRequest));
        }
        Ok(())
    }
}

pub struct Preparation {
    pub identity: Identity,
    pub route: Route,
    pub deadline: Instant,
    pub max_total_tokens: u32,
}

/// Explicit HTTP opt-in. Existing providers leave streaming disabled.
#[derive(Clone, Copy)]
pub struct StreamingPolicy {
    pub raw: RawStreamLimits,
    pub composed: ComposedStreamLimits,
    /// Optional comments between composed SSE frames after real headers.
    /// Raw passthrough never injects comments into upstream byte fragments.
    /// Static comments do not allocate or enter a producer queue.
    pub heartbeat: Option<Duration>,
}

impl StreamingPolicy {
    pub fn validate(&self) -> Result<(), DispatchError> {
        if self.heartbeat.is_some_and(|period| period < Duration::from_millis(10)
            || period > Duration::from_secs(3600)) {
            return Err(DispatchError::new(FailureKind::InvalidRequest));
        }
        self.raw.validate()?;
        self.composed.validate()
    }
}

pub trait InvocationProvider: Send + Sync {
    fn policy(&self) -> Policy;
    fn streaming_policy(&self) -> Option<StreamingPolicy> { None }
    fn contains_account(&self, account: AccountId) -> bool;
    fn supports_pool_admission(&self) -> bool { false }
    /// Explicit opt-in: preserve the passed ownership through actual cleanup.
    /// Rejection must either start no work or retain it until cleanup completes.
    fn prepare_in_pool(&self, request: Preparation, permit: OwnedSemaphorePermit,
        admission: PoolAdmission) -> BoxFuture<'_, Result<Box<dyn OwnedInvocation>, DispatchError>> {
        drop((request, permit, admission));
        Box::pin(async { Err(DispatchError::new(FailureKind::Ineligible)) })
    }
    /// Retains the consumed capacity through actual transport cleanup.
    fn prepare(&self, request: Preparation, permit: OwnedSemaphorePermit)
        -> BoxFuture<'_, Result<Box<dyn OwnedInvocation>, DispatchError>>;
}

/// A capability can begin exactly one logical invocation.
pub trait OwnedInvocation: Send {
    /// Compose downstream frames using the caller's display model without
    /// changing the upstream payload. Providers must explicitly support this
    /// contract; falling back to the wire-model method could lose the alias.
    fn composed_stream_for_client(self: Box<Self>, payload: Value,
        options: ComposedStreamOptions, client_model: String) -> ComposedStreamRun {
        drop((self, payload, options, client_model));
        ComposedStreamRun::unsupported()
    }
    fn composed_stream(self: Box<Self>, payload: Value, options: ComposedStreamOptions)
        -> ComposedStreamRun {
        drop((self, payload, options));
        ComposedStreamRun::unsupported()
    }
    fn raw(self: Box<Self>, payload: Value) -> InvocationRun<RawResult>;
    /// Optional raw byte streaming. Existing providers remain unsupported.
    fn raw_stream(self: Box<Self>, payload: Value, limits: RawStreamLimits) -> RawStreamRun {
        drop((self, payload, limits));
        RawStreamRun::unsupported()
    }
    fn messages(self: Box<Self>, payload: Value)
        -> InvocationRun<Completed<AssembledResponse>>;
    fn responses(self: Box<Self>, payload: Value)
        -> InvocationRun<Completed<AssembledResponses>>;
}

pub trait Cancellation: Send + Sync { fn cancel(&self); }
pub type Cleanup = Shared<BoxFuture<'static, Result<(), FailureKind>>>;
pub type AttemptProgress = Arc<dyn Fn() -> Vec<AttemptMetadata> + Send + Sync>;

#[must_use = "consume or drop the result and retain independent cleanup"]
pub struct InvocationRun<T> {
    pub result: BoxFuture<'static, Result<T, DispatchError>>,
    pub cleanup: Cleanup,
    pub cancellation: Arc<dyn Cancellation>,
    /// Snapshot retained if the public deadline wins a result-delivery race.
    pub completed_attempts: Option<AttemptProgress>,
}

#[derive(Clone)]
pub struct AttemptMetadata {
    pub status: u16,
    pub usage_header: Option<String>,
    pub original_content_type: Option<String>,
    pub step_index: u32,
    pub elapsed: Duration,
}

pub struct Completed<T> { pub value: T, pub attempts: Vec<AttemptMetadata> }
pub struct RawResult { pub body: Vec<u8>, pub metadata: AttemptMetadata }
/// Preserve this body owner when handing an upstream refusal to HTTP. In the
/// composed path it retains the JSON allocation's budget lease until its last
/// owning Bytes handle is dropped. Copying to Vec loses that ownership link.
pub struct UpstreamFailure { pub body: bytes::Bytes, pub metadata: AttemptMetadata }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    InvalidRequest, UnsupportedStreaming, UnsupportedResponse, MissingContext,
    MissingAuthorization, InvalidScope, ScopeClosed, Deadline, Cancelled,
    Overloaded, Authorization, Transport, Body, BodyTooLarge, Upstream,
    TerminalFailure, Cleanup, Internal, Ineligible, PaidDisabled, CreditFloor,
}

pub struct DispatchError {
    pub kind: FailureKind,
    pub upstream: Option<UpstreamFailure>,
    pub attempts: Vec<AttemptMetadata>,
}

impl DispatchError {
    pub fn new(kind: FailureKind) -> Self {
        Self { kind, upstream: None, attempts: Vec::new() }
    }
}

impl std::fmt::Debug for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchError").field("kind", &self.kind)
            .field("has_upstream", &self.upstream.is_some())
            .field("completed_attempts", &self.attempts.len()).finish()
    }
}
