//! Supplied-token browser transport. Minting remains with the request owner.
use super::http::NativeCancellation;
use framer_core::ids::AccountId;
use futures::future::BoxFuture;

#[cfg(feature = "browser-transport")]
#[path = "browser_host.rs"]
mod host;
#[cfg(feature = "browser-transport")]
pub use host::BrowserHost;

/// Private, single-use input. Never include this value in diagnostics.
pub struct BrowserRequest {
    pub account: AccountId,
    pub user: String,
    pub cookie_header: String,
    pub device: String,
    pub path: String,
    pub bearer: String,
    pub wire: Vec<u8>,
    pub proxy: Option<String>,
    pub expires_unix_ms: u64,
    pub timeout_ms: u64,
    pub response_limit: usize,
}

pub struct BrowserResponse {
    pub status: u16,
    pub usage: Option<String>,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

pub struct BrowserOutcome {
    pub response: Result<BrowserResponse, &'static str>,
    pub cleanup_confirmed: bool,
}

pub trait BrowserTransport: Send + Sync {
    /// Warm the sender after native history ownership is established.
    /// Its owner remains responsible for shutdown if this waiter is cancelled.
    fn prepare(&self) -> BoxFuture<'static, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    fn send(&self, request: BrowserRequest, cancellation: NativeCancellation)
        -> BoxFuture<'static, BrowserOutcome>;
}
