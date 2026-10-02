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

/// Device identity: Castle keys every request token to a stable visitor id it
/// stores as `__cuid` in BOTH a cookie (`.framer.com`) and localStorage
/// (observed live: a UUIDv4, identical value in both stores; the token's
/// 8-char prefix stays constant per device while the payload is unique per
/// mint). If the webview's storage is ephemeral, Castle mints a NEW device id
/// every run -> the account looks like a new device each time -> bad risk
/// score. The host MUST eval this snippet once per page load BEFORE the first
/// MINT_SNIPPET call, with the account's persisted `castle_device_id`
/// (ManagedAccount::castle_device_id). `{device_id}` = the UUID.
#[allow(dead_code)]
pub const SEED_DEVICE_SNIPPET: &str = r#"(() => {
  const id = "{device_id}";
  if (localStorage.getItem("__cuid") !== id) localStorage.setItem("__cuid", id);
  if (!document.cookie.includes("__cuid=" + id)) {
    document.cookie = "__cuid=" + id + "; domain=.framer.com; path=/; max-age=31536000; SameSite=Lax";
  }
  return id;
})()"#;

#[allow(dead_code)] // kept as the documented mint contract; AI calls now mint
// in-page inside ai_post (token+transport must share one environment).
pub async fn mint(bridge: &dyn BrowserBridge, account: &AccountId) -> Result<CastleToken, String> {
    bridge.castle_token(account).await
}
