use async_trait::async_trait;
use bytes::Bytes;
use serde_json::Value;

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
    fn check_cancelled(&self) -> Result<(), String> {
        if self.is_cancelled() { Err("client cancelled".into()) } else { Ok(()) }
    }
}

/// One request's authorized Responses route, bound before policy execution.
/// Implementations retain authorization, retries, refunds and cleanup ownership.
#[async_trait]
pub trait AuthorizedForward: ChainControl {
    fn category(&self) -> &str;
    async fn forward(&self, step: &str, payload: Value, step_index: u32)
        -> Result<ForwardReply, String>;
}
