use async_trait::async_trait;
use bytes::Bytes;
use serde_json::Value;
use crate::forward_error::{ControlError, ErrorClass, ForwardError, PolicyError};

#[derive(Clone, Copy, Debug)]
pub enum PolicyDiag {
    ChunkStart { chunk: u32, max_tokens: u32 },
    Backoff { chunk: u32, attempt: u32, wait_ms: u64 },
    RetryHalved { chunk: u32, elapsed_ms: u64, max_tokens: u32, cause: ErrorClass },
    MeteredFallback { chunk: u32, cause: ErrorClass },
    UpstreamQueued { chunk: u32, queue_ms: u64, wall_ms: u64 },
    ChunkDone { chunk: u32, tokens: u64, elapsed_ms: u64 },
    ToolBudgetWithheld { chunk: u32 },
    ToolRetry { chunk: u32, max_tokens: u32 },
    NamespaceBackfill { count: u64 },
}

/// Per-request policy settings. Runtime callbacks stay behind ChainControl.
#[derive(Clone, Debug)]
pub struct ChainLimits {
    pub chunk_tokens: u32,
    pub max_chunks: u32,
    pub continue_prompt: String,
    pub max_total_tokens: Option<u32>,
    pub metered_fallback: bool,
}

/// A completed, buffered upstream operation, without runtime credential types.
pub struct ForwardReply {
    pub status: u16,
    pub content_type: String,
    pub body: Bytes,
    pub queue_wait_ms: u64,
}

/// Runtime-owned cancellation and saturation, called at policy decision points.
pub trait ChainControl: Send + Sync {
    fn is_cancelled(&self) -> bool;
    fn on_429(&self);
    fn diag(&self, _event: PolicyDiag) {}
    fn check_cancelled(&self) -> Result<(), String> {
        if self.is_cancelled() { Err("client cancelled".into()) } else { Ok(()) }
    }
}

/// One request's authorized Responses route, bound before policy execution.
/// Implementations retain authorization, retries, refunds and cleanup ownership.
#[async_trait]
pub trait AuthorizedForward: ChainControl {
    fn category(&self) -> &str;
    /// Returns only after the runtime applies the effect. Failure is terminal
    /// for policy execution, even when an upstream retry would be allowed.
    async fn saturated(&self) -> Result<(), ControlError> {
        self.on_429();
        Ok(())
    }
    /// IPC implementations override this with runtime-classified facts.
    /// The compatibility default keeps the original local display string.
    async fn forward_typed(&self, step: &str, payload: Value, step_index: u32)
        -> Result<ForwardReply, PolicyError> {
        self.forward(step, payload, step_index).await
            .map_err(|message| ForwardError::from_legacy(message).into())
    }
    async fn forward(&self, step: &str, payload: Value, step_index: u32)
        -> Result<ForwardReply, String>;
}
