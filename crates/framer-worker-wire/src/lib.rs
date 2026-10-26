//! Bounded transport primitives. This crate has no application or account ports.
pub mod credit;
pub mod frame;
pub mod message;

pub const VERSION: u16 = 1;
pub const FRAME_LIMIT: usize = 1024 * 1024;
pub const SEGMENT_LIMIT: usize = 16 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("worker transport I/O failed")]
    Io(#[from] std::io::Error),
    #[error("worker frame encoding is invalid")]
    Json(#[from] serde_json::Error),
    #[error("worker frame exceeds its byte limit")]
    FrameLimit,
    #[error("worker frame allocation failed")]
    Allocation,
    #[error("worker credit or event sequence is invalid")]
    Credit,
    #[error("worker protocol state is invalid")]
    Protocol,
}
