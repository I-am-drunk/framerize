//! Credit / workspace policy. The single place that turns "out of credits" (402)
//! into "switch workspace and keep going".
//!
//! Framer credits are WORKSPACE-scoped (per team), so rotation means switching
//! `current_team` to another team the account owns that still has credits —
//! or creating a fresh one (endpoint not yet captured, see framer-api/workspace.rs).

use crate::account::ManagedAccount;
use crate::error::CoreError;
use crate::ids::TeamId;
use crate::pipeline::{default_workspace_spec, Deps};
use crate::port::Session;

/// Ensure the account points at a credited workspace. Cheap path: keep the current
/// team if its cached snapshot still shows credits.
pub async fn ensure_workspace(
    deps: &Deps<'_>,
    account: &mut ManagedAccount,
    session: &Session,
) -> Result<TeamId, CoreError> {
    if let (Some(team), Some(credit)) = (&account.current_team, &account.credit) {
        if !credit.looks_exhausted() {
            return Ok(team.clone());
        }
    }
    rotate_workspace(deps, account, session).await
}

/// Point the account at another of its teams with credits, else create one.
pub async fn rotate_workspace(
    deps: &Deps<'_>,
    account: &mut ManagedAccount,
    session: &Session,
) -> Result<TeamId, CoreError> {
    // 1. Existing teams with credits (from /web/users/me spaces[].ownerId).
    let me = deps.client.me(session).await.map_err(CoreError::Framer)?;
    if let Some(spaces) = me["spaces"].as_array() {
        for space in spaces {
            let Some(owner) = space["ownerId"].as_str() else { continue };
            let team = TeamId(owner.to_string());
            if Some(&team) == account.current_team.as_ref() {
                continue;
            }
            if let Ok(snap) = deps.client.usage(session, &team).await {
                if !snap.looks_exhausted() {
                    account.current_team = Some(team.clone());
                    account.credit = Some(snap);
                    deps.state.upsert(account).await.map_err(CoreError::Store)?;
                    return Ok(team);
                }
            }
        }
    }

    // 2. Fresh workspace (create-team endpoint not yet captured — see port docs).
    let team = deps
        .client
        .create_workspace(session, &default_workspace_spec())
        .await
        .map_err(CoreError::Framer)?;
    account.current_team = Some(team.clone());
    account.credit = None;
    deps.state.upsert(account).await.map_err(CoreError::Store)?;
    Ok(team)
}
