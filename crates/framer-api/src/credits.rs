//! Credit ledger reads. Two sources:
//!     - GET /web/teams/{teamId}/ai-credits  (web Bearer JWT) — the full ledger.
//!     - X-Framer-AI-Usage response header on every successful /ai/v3/* call —
//!     free, poll-free; parse and cache it after every run.
//! Credits are workspace(team)-scoped. See reference/analysis/framer-credits.md.

use crate::auth;
use crate::transport::{Auth, Transport};
use framer_core::ids::TeamId;
use framer_core::port::Session;
use framer_core::workspace::CreditSnapshot;

/// GET /web/users/me — profile + workspaces. `spaces[].ownerId` is the
/// credit-bearing team for each workspace space.
pub async fn me(t: &Transport, session: &Session) -> Result<serde_json::Value, String> {
    t.get("/web/users/me", Auth::Cookie(session))
        .await?
        .json()
        .await
        .map_err(|e| e.to_string())
}

pub async fn usage(
    t: &Transport,
    session: &Session,
    team: &TeamId,
) -> Result<CreditSnapshot, String> {
    let web = auth::web_access_token(t, session).await?;
    let v = t
        .get(&format!("/web/teams/{}/ai-credits", team.0), Auth::Bearer(&web.bearer))
        .await?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?;
    Ok(CreditSnapshot {
        available: v["availableCredits"].as_f64().unwrap_or(0.0),
        funds: v["funds"].as_array().map(|a| a.len() as u32).unwrap_or(0),
    })
}

/// `X-Framer-AI-Usage` is URLSearchParams ("remaining=484.875&…").
pub fn parse_usage_header(value: &str) -> Option<f64> {
    value.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        if k == "remaining" { v.parse().ok() } else { None }
    })
}
