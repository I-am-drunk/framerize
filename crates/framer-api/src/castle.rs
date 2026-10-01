//! Castle request tokens cannot be minted in Rust — they fingerprint a real
//! browser engine. This module documents the bridge contract; the implementation
//! is JS evaluated inside the account's isolated WebKit session:
//!
//! ```js
//! const m = await import("<castle chunk, e.g. chunk-5LKSDTQ5.mjs>");
//! return await m.a();  // ~5.9 KB "cNz8rPYH|…" token; single-shot, ~seconds of life
//! ```
//!
//! The chunk hash changes on deploy. Resolution order:
//!     1. the pinned constant below (works until Framer redeploys);
//!     2. discover at runtime: list chunk-*.mjs resources on the editor page and pick
//!      the one containing "castleSDKV3".
//! See reference/analysis/framer-castle-token.md.

use framer_core::ids::AccountId;
use framer_core::port::{BrowserBridge, CastleToken};

#[allow(dead_code)] // documentation constant — used by the webview host
pub const CASTLE_CHUNK_PINNED: &str = "https://app.framerstatic.com/chunk-5LKSDTQ5.mjs";
#[allow(dead_code)]
pub const CASTLE_SDK_V3: &str = "https://app.framerstatic.com/castleSDKV3-LNGA36I3.mjs";

/// JS snippet to eval inside the account webview. `{chunk}` = CASTLE_CHUNK_PINNED
/// or a discovered chunk URL.
#[allow(dead_code)]
pub const MINT_SNIPPET: &str = r#"import("{chunk}").then(m => m.a())"#;

pub async fn mint(bridge: &dyn BrowserBridge, account: &AccountId) -> Result<CastleToken, String> {
    bridge.castle_token(account).await
}
