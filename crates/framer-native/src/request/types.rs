use futures::future::BoxFuture;
use futures::future::Shared;
use std::sync::Arc;
use std::time::Duration;
pub use framer_core::admission::Identity;
pub use framer_core::admission::PoolAdmission;
pub trait Cancellation: Send + Sync { fn cancel(&self); }
pub type Cleanup = Shared<BoxFuture<'static, Result<(), FailureKind>>>;
pub struct InvocationRun<T> { pub result: BoxFuture<'static, Result<T, DispatchError>>, pub cleanup: Cleanup, pub cancellation: Arc<dyn Cancellation> }
#[derive(Clone)]
pub struct AttemptMetadata {
    pub status: u16,
    pub usage_header: Option<String>,
    pub original_content_type: Option<String>,
    pub step_index: u32,
    pub elapsed: Duration,
}

pub struct Completed<T> { pub value: T, pub attempts: Vec<AttemptMetadata> }
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

impl From<crate::service::admission::AdmissionError> for FailureKind {
    fn from(error: crate::service::admission::AdmissionError) -> Self {
        use crate::service::admission::AdmissionError as E;
        match error {
            E::InvalidRequest => Self::InvalidRequest,
            E::InvalidScope => Self::InvalidScope,
            E::Ineligible => Self::Ineligible,
            E::Overloaded => Self::Overloaded,
            E::Deadline => Self::Deadline,
            E::Cleanup => Self::Cleanup,
            E::Internal => Self::Internal,
        }
    }
}
