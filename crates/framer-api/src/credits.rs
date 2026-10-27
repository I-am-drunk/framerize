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

#[cfg(test)]
#[path = "credit_ledger_validation_tests.rs"]
mod ledger_validation_tests;

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
    let available = v["availableCredits"]
        .as_f64()
        .filter(|available| available.is_finite())
        .ok_or("bad credit ledger response: invalid availableCredits")?;
    // One fund can hold MANY grants: a 1000-credit account read as
    // {funds:1, availableCredits:1000} is two 500 grants stacked in one fund
    // (reference/analysis/framer-credits.md:61-74 shows the nesting). Counting
    // funds alone hid that, which made "how many project grants does this
    // account have?" unanswerable. Count grants too.
    let grants: u32 = v["funds"]
        .as_array()
        .map(|funds| {
            funds
                .iter()
                .map(|f| f["grants"].as_array().map(|g| g.len() as u32).unwrap_or(0))
                .sum()
        })
        .unwrap_or(0);
    let allocated: f64 = v["funds"]
        .as_array()
        .map(|funds| {
            funds
                .iter()
                .flat_map(|f| f["grants"].as_array().cloned().unwrap_or_default())
                .map(|g| g["allocatedCredits"].as_f64().unwrap_or(0.0))
                .sum()
        })
        .unwrap_or(0.0);

    Ok(CreditSnapshot {
        available,
        funds: v["funds"].as_array().map(|a| a.len() as u32).unwrap_or(0),
        grants,
        allocated,
    })
}

/// `X-Framer-AI-Usage` is URLSearchParams ("remaining=484.875&…").
pub fn parse_usage_header(value: &str) -> Option<f64> {
    value.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        if k == "remaining" { v.parse().ok() } else { None }
    })
}
