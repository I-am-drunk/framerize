//! Additive raw streaming capability. No policy defaults or runtime enablement.
use super::{Cancellation, DispatchError, FailureKind, InvocationRun};
use bytes::Bytes;
use futures::{future::{BoxFuture, Shared}, FutureExt, Stream};
use std::{pin::Pin, sync::Arc, time::Duration};

/// Explicit payload budgets. There is no adapter queue or producer body pump.
/// Reserve chunk_bytes before polling upstream; each returned Bytes retains
/// that reservation through its last clone/slice. At most retained_bytes /
/// chunk_bytes allocations can be outstanding. Typed/allocator overhead and
/// native transport storage are separate from these budgets. Native length
/// limits alone do not establish the size of shared backing allocations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawStreamLimits {
    pub chunk_bytes: usize,
    pub retained_bytes: usize,
    pub total_bytes: usize,
    /// Original header strings shared by raw header/terminal observers. Legacy
    /// error/progress snapshots are explicit caller-owned copies.
    pub header_bytes: usize,
}

impl RawStreamLimits {
    pub fn validate(self) -> Result<(), DispatchError> {
        const MAX_BODY: usize = 64 * 1024 * 1024;
        if self.chunk_bytes == 0 || self.chunk_bytes > self.retained_bytes
            || self.chunk_bytes > self.total_bytes || self.retained_bytes > MAX_BODY
            || self.total_bytes > MAX_BODY || self.header_bytes == 0
            || self.header_bytes > 64 * 1024 {
            return Err(DispatchError::new(FailureKind::InvalidRequest));
        }
        Ok(())
    }
}

/// Actual upstream headers. Missing content type uses only the supplied policy.
/// elapsed is time to headers, not body or cleanup completion. Strings share
/// their backing storage across header and terminal observers.
pub struct RawStreamHeaders {
    pub status: u16,
    pub usage_header: Option<Arc<str>>,
    pub original_content_type: Option<Arc<str>>,
    pub content_type: Arc<str>,
    pub elapsed: Duration,
}

pub type RawStreamBody = Pin<Box<dyn Stream<Item = Result<Bytes, FailureKind>> + Send>>;

#[must_use = "consume or drop the body; observe finished and cleanup separately"]
pub struct RawStreamHead {
    pub headers: Arc<RawStreamHeaders>,
    pub body: RawStreamBody,
}

/// Application body outcome. Failures do not wait for independent cleanup.
/// Success requires body EOF and real native completion; it does not mean
/// remote delivery or upstream semantic success. Outstanding Bytes may still
/// hold cleanup open after body EOF or cancellation.
#[derive(Clone)]
pub struct RawStreamCompletion {
    pub result: Result<(), FailureKind>,
    pub headers: Option<Arc<RawStreamHeaders>>,
    pub emitted_bytes: usize,
    pub elapsed: Duration,
}

pub type RawStreamFinished = Shared<BoxFuture<'static, RawStreamCompletion>>;

#[must_use = "retain independent finished and cleanup observers"]
pub struct RawStreamRun {
    pub response: InvocationRun<RawStreamHead>,
    pub finished: RawStreamFinished,
}

struct NoWork;
impl Cancellation for NoWork { fn cancel(&self) {} }

impl RawStreamRun {
    pub(super) fn unsupported() -> Self {
        Self {
            response: InvocationRun {
                result: async { Err(DispatchError::new(FailureKind::UnsupportedStreaming)) }.boxed(),
                cleanup: async { Ok(()) }.boxed().shared(),
                cancellation: Arc::new(NoWork), completed_attempts: None,
            },
            finished: async { RawStreamCompletion {
                result: Err(FailureKind::UnsupportedStreaming), headers: None,
                emitted_bytes: 0, elapsed: Duration::ZERO,
            } }.boxed().shared(),
        }
    }
}
