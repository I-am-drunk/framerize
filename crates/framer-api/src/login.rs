//! Login = email magic link. Verified against login.36HT4Q4D.mjs:
//!
//! ```text
//!     1. POST /auth/v4/signin   {email, castleToken} → emails the link.
//!      (redirect/inviteId/origin are omitted by the SPA for a plain login —
//!      sending them as null plus forceMagic gets a 400. Observed live.)
//!      403 here = "VERIFICATION_FAILURE" (Castle refused the sign-in attempt).
//!     2. User clicks the emailed link: /login#token=<uuid>. The SPA strips the hash
//!      and posts {token, castleToken} — BOTH fields required — to:
//!      POST /auth/v4/continue → Set-Cookie: session=… (.framer.com, ~1y).
//!
//! Links expire after 1 hour and are single-use. Both Castle tokens must be fresh,
//! minted in the account's own webview (castle.rs).
//! ```

use crate::transport::{Auth, Transport};
use framer_core::ids::FramerUserId;
use framer_core::port::{CastleToken, Session};
use serde_json::json;

pub async fn request_magic_link(
    t: &Transport,
    email: &str,
    castle: &CastleToken,
) -> Result<(), String> {
    // Matches the web login bundle exactly (login.36HT4Q4D.mjs):
    //   Je({email, redirect: U, inviteId: f, origin: W, castleToken})
    // redirect/inviteId come from URL params → undefined → dropped by
    // JSON.stringify. origin is NEVER undefined: Qr() → "web" on desktop
    // ("web-mobile" on mobile UA, "web-onpage" for the ?close=true flow).
    // And every API call needs the X-Requested-By: Framer header
    // (transport adds it). Bare {email, castleToken} or null fields → 400.
    t.post_json(
        "/auth/v4/signin",
        Auth::None,
        json!({
            "email": email,
            "origin": "web",
            "castleToken": castle.0,
        }),
    )
    .await?;
    Ok(())
}

pub async fn complete_magic_link(
    t: &Transport,
    email_token: &str,
    castle: &CastleToken,
) -> Result<Session, String> {
    let resp = t
        .post(
            "/auth/v4/continue",
            Auth::None,
            json!({ "token": email_token, "castleToken": castle.0 }),
        )
        .await?;

    // The session arrives as Set-Cookie, not in the body.
    let cookie = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|h| h.to_str().ok())
        .find(|h| h.starts_with("session="))
        .and_then(|h| h.split(';').next())
        .map(str::to_string)
        .ok_or("no session cookie in continue response")?;

    // Resolve the user id for keychain keying. /web/users/me is cookie-authed.
    let probe = Session {
        active_user: FramerUserId(String::new()),
        cookie_header: cookie.clone(),
    };
    let me = t.get("/web/users/me", Auth::Cookie(&probe)).await?;
    let v: serde_json::Value = me.json().await.map_err(|e| e.to_string())?;
    let user = v["id"].as_str().ok_or("bad /web/users/me response")?;

    Ok(Session {
        active_user: FramerUserId(user.to_string()),
        cookie_header: cookie,
    })
}
