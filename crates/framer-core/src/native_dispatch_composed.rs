//! Opt-in composed streams. No deployment defaults or upstream policy.
use super::{Cancellation, DispatchError, FailureKind, InvocationRun, RawStreamBody, RawStreamHeaders};
use crate::convert::ClientDialect;
use futures::{future::{BoxFuture, Shared}, FutureExt};
use std::{sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug)]
pub struct ComposedStreamOptions {
    pub dialect: ClientDialect,
    pub include_usage: bool,
    pub limits: ComposedStreamLimits,
}

#[derive(Clone)]
pub struct ComposedAttemptMetadata {
    pub headers: Arc<RawStreamHeaders>,
    /// The original provider response ID for this attempt, when parsed.
    pub response_id: Option<Arc<str>>,
    /// Exact per-attempt provider usage, distinct from aggregate token counts.
    pub usage: Option<Arc<serde_json::Value>>,
    pub step_index: u32,
    pub elapsed: Duration,
}

#[must_use = "consume or drop the body and retain independent cleanup"]
pub struct ComposedStreamHead {
    pub first_attempt: Arc<RawStreamHeaders>,
    pub content_type: &'static str,
    pub body: RawStreamBody,
}

#[derive(Clone)]
pub struct ComposedStreamCompletion {
    pub result: Result<(), FailureKind>,
    pub attempts: Arc<[ComposedAttemptMetadata]>,
    /// Bytes yielded to the consumer, not proof of remote network delivery.
    pub emitted_bytes: usize,
    pub elapsed: Duration,
}

pub type ComposedStreamFinished = Shared<BoxFuture<'static, ComposedStreamCompletion>>;

#[must_use = "retain independent finished and cleanup observers"]
pub struct ComposedStreamRun {
    pub response: InvocationRun<ComposedStreamHead>,
    pub finished: ComposedStreamFinished,
}

struct NoWork;
impl Cancellation for NoWork { fn cancel(&self) {} }

impl ComposedStreamRun {
    pub(super) fn unsupported() -> Self {
        Self {
            response: InvocationRun {
                result: async { Err(DispatchError::new(FailureKind::UnsupportedStreaming)) }.boxed(),
                cleanup: async { Ok(()) }.boxed().shared(),
                cancellation: Arc::new(NoWork), completed_attempts: None,
            },
            finished: async { ComposedStreamCompletion {
                result: Err(FailureKind::UnsupportedStreaming), attempts: Arc::from([]),
                emitted_bytes: 0, elapsed: Duration::ZERO,
            } }.boxed().shared(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComposedStreamLimits {
    /// One frame allocation. Chat's complete success suffix shares one such
    /// allocation and must fit before any success marker can be enqueued.
    pub frame_bytes: usize,
    /// Covers allocations through their last output clone or slice.
    pub retained_bytes: usize,
    /// Logical SSE frames, including each frame in a shared Chat suffix.
    /// Chat needs at least three when include_usage is true, otherwise two.
    pub retained_frames: usize,
    /// Conservative cumulative JSON/history/composer allocation admission,
    /// including local request metadata copies. Never refunded. Excludes
    /// shared account/transport state, allocator metadata, supplier-owned
    /// storage and caller-retained legacy progress snapshots.
    pub state_bytes: usize,
    /// One admitted, fixed per-attempt JSON wire collection allocation. A
    /// returned refusal retains this allowance through its last owning Bytes
    /// handle, independently of the SSE frame allowance in retained_bytes.
    pub json_bytes: usize,
    pub source_chunk_bytes: usize,
    /// Sum of original headers across attempts, including fallback strings.
    pub header_bytes: usize,
    pub total_bytes: usize,
    /// Separate error-frame allocation, retained through its last Bytes owner.
    pub terminal_bytes: usize,
}

impl ComposedStreamLimits {
    pub fn validate(self) -> Result<(), DispatchError> {
        const MAX: usize = 64 * 1024 * 1024;
        if self.frame_bytes == 0 || self.frame_bytes > self.retained_bytes
            || self.retained_bytes > MAX || !(1..=1024).contains(&self.retained_frames)
            || self.state_bytes == 0 || self.state_bytes > MAX
            || self.json_bytes == 0 || self.json_bytes > MAX
            || self.source_chunk_bytes == 0 || self.source_chunk_bytes > self.json_bytes
            || self.source_chunk_bytes > 1024 * 1024
            || self.header_bytes == 0 || self.header_bytes > 64 * 1024
            || self.total_bytes < self.frame_bytes || self.total_bytes > MAX
            || !(512..=64 * 1024).contains(&self.terminal_bytes)
            || self.frame_bytes.checked_add(self.terminal_bytes)
                .is_none_or(|n| n > self.total_bytes) {
            return Err(DispatchError::new(FailureKind::InvalidRequest));
        }
        Ok(())
    }
}
