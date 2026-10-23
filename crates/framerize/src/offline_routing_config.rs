//! Explicit routing limits. No production metadata or credit is invented.
use framer_core::native_dispatch::{Policy, StreamingPolicy, RawStreamLimits, ComposedStreamLimits};
use serde::Deserialize;
use std::time::Duration;

#[cfg(test)]
#[path = "offline_routing_config_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "offline_routing_example_tests.rs"]
mod example_tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    timeout_ms: u64,
    max_body_bytes: usize,
    missing_content_type: String,
    chunk_tokens: u32,
    max_chunks: u32,
    continuation: Option<String>,
    allow_paid: bool,
    credit_floor: f64,
    streaming: Option<Streaming>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Streaming {
    raw: Raw,
    composed: Composed,
    heartbeat_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    chunk_bytes: usize,
    retained_bytes: usize,
    total_bytes: usize,
    header_bytes: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Composed {
    frame_bytes: usize,
    retained_bytes: usize,
    retained_frames: usize,
    state_bytes: usize,
    json_bytes: usize,
    source_chunk_bytes: usize,
    header_bytes: usize,
    total_bytes: usize,
    terminal_bytes: usize,
}

pub(crate) fn load(path: &str) -> Result<(Policy, Option<StreamingPolicy>), &'static str> {
    decode(&super::offline_config::read(path)?)
}

fn decode(bytes: &[u8]) -> Result<(Policy, Option<StreamingPolicy>), &'static str> {
    let c: Config = serde_json::from_slice(bytes).map_err(|_| "invalid offline routing config")?;
    if c.timeout_ms > 1_800_000 || c.continuation.as_ref().is_some_and(|s| s.len() > 4096) {
        return Err("offline routing policy exceeds bounds");
    }
    let policy = Policy { timeout: Duration::from_millis(c.timeout_ms),
        max_body_bytes: c.max_body_bytes, missing_content_type: c.missing_content_type,
        chunk_tokens: c.chunk_tokens, max_chunks: c.max_chunks, continuation: c.continuation,
        allow_paid: c.allow_paid, credit_floor: c.credit_floor };
    policy.validate().map_err(|_| "invalid offline routing policy")?;
    let streaming = c.streaming.map(streaming).transpose()?;
    Ok((policy, streaming))
}

fn streaming(s: Streaming) -> Result<StreamingPolicy, &'static str> {
    let raw = RawStreamLimits { chunk_bytes: s.raw.chunk_bytes,
        retained_bytes: s.raw.retained_bytes, total_bytes: s.raw.total_bytes,
        header_bytes: s.raw.header_bytes };
    let c = s.composed;
    let composed = ComposedStreamLimits { frame_bytes: c.frame_bytes,
        retained_bytes: c.retained_bytes, retained_frames: c.retained_frames,
        state_bytes: c.state_bytes, json_bytes: c.json_bytes,
        source_chunk_bytes: c.source_chunk_bytes, header_bytes: c.header_bytes,
        total_bytes: c.total_bytes, terminal_bytes: c.terminal_bytes };
    let policy = StreamingPolicy { raw, composed,
        heartbeat: s.heartbeat_ms.map(Duration::from_millis) };
    policy.validate().map_err(|_| "invalid offline streaming limits")?;
    Ok(policy)
}
