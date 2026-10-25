//! Credit / workspace policy. The single place that turns "out of credits" (402)
//! into "switch workspace and keep going".
//!
//! Framer credits are WORKSPACE-scoped (per team), so rotation means switching
//! the team/project pair to another credited team and its existing project.
//! Resolution must succeed before either member of that pair is published.

use crate::account::ManagedAccount;
use crate::error::CoreError;
use crate::ids::{ProjectId, TeamId};
use crate::pipeline::Deps;
use crate::port::Session;
use crate::workspace::CreditSnapshot;

/// Ensure the account points at a credited workspace. Cheap path: keep the current
/// team if its cached snapshot still shows credits.
pub async fn ensure_workspace(
    deps: &Deps<'_>,
    account: &mut ManagedAccount,
    session: &Session,
) -> Result<TeamId, CoreError> {
    if session.active_user != account.framer_user_id {
        return Err(CoreError::NotEligible("workspace session owner mismatch".into()));
    }
    if let Some(team) = account.current_team.clone() {
        match &account.credit {
            Some(credit) if !credit.looks_exhausted() => return Ok(team),
            _ => {
                // Missing (freshly imported account) or maybe-stale snapshot:
                // refresh the CURRENT team's ledger before rotating away from it.
                // (rotate_workspace deliberately skips the current team, so an
                // unrefreshed None used to fall straight into rotation and then
                // the create-team stub even when the current team had credits.)
                if let Ok(snap) = deps.client.usage(session, &team).await {
                    let ok = !snap.looks_exhausted();
                    deps.update(account, |current| current.credit = Some(snap)).await?;
                    if ok {
                        return Ok(team);
                    }
                }
            }
        }
    }
    rotate_workspace(deps, account, session).await
}

/// Select another credited team and resolve its existing project before publishing.
pub async fn rotate_workspace(
    deps: &Deps<'_>,
    account: &mut ManagedAccount,
    session: &Session,
) -> Result<TeamId, CoreError> {
    if session.active_user != account.framer_user_id {
        return Err(CoreError::NotEligible("workspace session owner mismatch".into()));
    }
    // Existing teams with credits (from /web/users/me spaces[].ownerId).
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
                    let project = existing_project(deps, account, session, &team).await?;
                    publish_workspace(deps, account, &team, project, snap).await?;
                    return Ok(team);
                }
            }
        }
    }

    // Creating a team cannot supply a verified existing project for that team.
    Err(CoreError::NotEligible("no other credited workspace with an existing project".into()))
}

async fn existing_project(
    deps: &Deps<'_>, account: &ManagedAccount, session: &Session, team: &TeamId,
) -> Result<ProjectId, CoreError> {
    // Extra scopes were resolved for this account during provisioning.
    if let Some(scope) = account.extra_workspaces.iter()
        .find(|scope| &scope.team == team && !scope.project.0.is_empty()) {
        return Ok(scope.project.clone());
    }
    deps.client.team_projects(session, team).await.map_err(CoreError::Framer)?
        .into_iter().find(|project| !project.0.is_empty())
        .ok_or_else(|| CoreError::NotEligible("credited workspace has no existing project".into()))
}

async fn publish_workspace(
    deps: &Deps<'_>, account: &mut ManagedAccount, team: &TeamId,
    project: ProjectId, snapshot: CreditSnapshot,
) -> Result<(), CoreError> {
    let _update = deps.account_updates.lock().await;
    let mut current = deps.state.get(account.id).await.map_err(CoreError::Store)?
        .ok_or_else(|| CoreError::NotEligible("account removed during rotation".into()))?;
    // Recheck the operation's identity and primary scope under the same lock as
    // health writers. Preserve unrelated scopes and current health fields.
    if current.id != account.id || current.framer_user_id != account.framer_user_id
        || current.castle_device_id != account.castle_device_id
        || current.current_team != account.current_team
        || current.current_project != account.current_project || current.proxy != account.proxy {
        return Err(CoreError::NotEligible("account scope changed during rotation".into()));
    }
    // Cached project authority must not outlive a concurrent scope replacement.
    if !current.extra_workspaces.iter().map(|scope| (&scope.team, &scope.project))
        .eq(account.extra_workspaces.iter().map(|scope| (&scope.team, &scope.project))) {
        return Err(CoreError::NotEligible("account workspace scopes changed during rotation".into()));
    }
    current.current_team = Some(team.clone());
    current.current_project = Some(project);
    current.credit = Some(snapshot);
    // The promoted team now has a primary slot, not a second round-robin slot.
    current.extra_workspaces.retain(|scope| &scope.team != team);
    deps.state.upsert(&current).await.map_err(CoreError::Store)?;
    *account = current;
    Ok(())
}

/// One workspace's credit verdict.
#[derive(Debug, Clone)]
pub struct WorkspaceCredit {
    pub team: TeamId,
    pub snapshot: crate::workspace::CreditSnapshot,
}

/// Scan EVERY workspace the account owns, concurrently, and return the credited
/// ones (richest first).
///
/// Why concurrent: `rotate_workspace` checks teams one at a time and stops at
/// the first hit, so admitting an account with N workspaces costs N sequential
/// round trips -- and each `usage` call is a real HTTP request. Fanning out
/// turns an N-request wait into one round trip's latency, which is what makes
/// "is this account useful at all?" answerable in well under a second.
///
/// An account whose every workspace is exhausted cannot serve a single request,
/// so admitting it means rotation sends it traffic that is guaranteed to fail
/// (the 2026-10-04 lesson, in a different costume).
pub async fn scan_workspaces(
    deps: &Deps<'_>,
    session: &Session,
) -> Result<Vec<WorkspaceCredit>, CoreError> {
    let me = deps.client.me(session).await.map_err(CoreError::Framer)?;

    let teams: Vec<TeamId> = me["spaces"]
        .as_array()
        .map(|spaces| {
            spaces
                .iter()
                .filter_map(|s| s["ownerId"].as_str())
                .map(|id| TeamId(id.to_string()))
                .collect()
        })
        .unwrap_or_default();

    if teams.is_empty() {
        return Ok(Vec::new());
    }

    // Fan out. A failed probe is NOT evidence of exhaustion -- it is evidence of
    // nothing -- so it is dropped rather than counted as "no credits".
    let probes = teams.into_iter().map(|team| async move {
        match deps.client.usage(session, &team).await {
            Ok(snapshot) if !snapshot.looks_exhausted() => Some(WorkspaceCredit { team, snapshot }),
            _ => None,
        }
    });

    let mut credited: Vec<WorkspaceCredit> =
        futures::future::join_all(probes).await.into_iter().flatten().collect();

    // Richest first so the caller lands on the workspace that will last longest.
    credited.sort_by(|a, b| b.snapshot.available.total_cmp(&a.snapshot.available));
    Ok(credited)
}

/// Admission: the account must bring a NEW credited workspace, or it is refused.
///
/// "Credited" alone is not enough. Credits and the 429 concurrency cap are both
/// WORKSPACE-scoped (reference/analysis/framer-credits.md; AGENTS.md: "credits
/// live here, not on the account"), so an account landing on a workspace some
/// other account already uses adds **no capacity at all** -- it splits one
/// credit pool and one concurrency budget across two records while looking like
/// growth. Only a workspace nobody has claimed is real capacity.
///
/// `claimed` is the set of team ids already in use by OTHER accounts; the
/// caller excludes this account's own id so a re-login keeps its workspace.
pub async fn admit(
    deps: &Deps<'_>,
    account: &mut ManagedAccount,
    session: &Session,
    claimed: &std::collections::BTreeSet<String>,
) -> Result<TeamId, CoreError> {
    let who = account.email.clone().unwrap_or_else(|| account.alias.clone());
    let credited = scan_workspaces(deps, session).await?;

    if credited.is_empty() {
        return Err(CoreError::NotEligible(format!(
            "{who}: no workspace has credits — account cannot serve requests",
        )));
    }

    // Richest-first among the workspaces nobody else holds.
    let Some(fresh) = credited.iter().find(|w| !claimed.contains(&w.team.0)) else {
        let names: Vec<&str> = credited.iter().map(|w| w.team.0.as_str()).collect();
        return Err(CoreError::NotEligible(format!(
            "{who}: every credited workspace is already in use ({}) — adding this              account would split one credit pool and one concurrency budget, not              add capacity. Create a new workspace for it first.",
            names.join(", "),
        )));
    };

    deps.update(account, |current| {
        current.current_team = Some(fresh.team.clone());
        current.credit = Some(fresh.snapshot);
    }).await?;
    Ok(fresh.team.clone())
}

#[cfg(test)]
#[path = "credits_rotation_tests.rs"]
mod rotation_tests;
