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
                    account.credit = Some(snap);
                    deps.state.upsert(account).await.map_err(CoreError::Store)?;
                    if ok {
                        return Ok(team);
                    }
                }
            }
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

    account.current_team = Some(fresh.team.clone());
    account.credit = Some(fresh.snapshot.clone());
    deps.state.upsert(account).await.map_err(CoreError::Store)?;
    Ok(fresh.team.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::CreditSnapshot;
    use std::collections::BTreeSet;

    fn wc(team: &str, available: f64) -> WorkspaceCredit {
        WorkspaceCredit {
            team: TeamId(team.to_string()),
            snapshot: CreditSnapshot { available, funds: 1, grants: 1, allocated: available },
        }
    }

    /// The selection rule `admit` applies, isolated from the network.
    fn choose<'a>(
        credited: &'a [WorkspaceCredit],
        claimed: &BTreeSet<String>,
    ) -> Option<&'a WorkspaceCredit> {
        credited.iter().find(|w| !claimed.contains(&w.team.0))
    }

    #[test]
    fn an_unclaimed_workspace_is_accepted() {
        let credited = vec![wc("fresh", 500.0)];
        let claimed = BTreeSet::new();
        assert_eq!(choose(&credited, &claimed).unwrap().team.0, "fresh");
    }

    #[test]
    fn a_workspace_another_account_already_holds_is_refused() {
        // The whole point: credits and the 429 cap are workspace-scoped, so
        // landing on a claimed workspace splits one budget instead of adding one.
        let credited = vec![wc("shared", 500.0)];
        let claimed: BTreeSet<String> = ["shared".to_string()].into();
        assert!(choose(&credited, &claimed).is_none());
    }

    #[test]
    fn the_first_unclaimed_workspace_wins_even_if_poorer() {
        // scan_workspaces sorts richest-first, so "first unclaimed" is the best
        // available -- but a claimed richer one must never be chosen.
        let credited = vec![wc("rich-but-taken", 900.0), wc("poorer-free", 10.0)];
        let claimed: BTreeSet<String> = ["rich-but-taken".to_string()].into();
        assert_eq!(choose(&credited, &claimed).unwrap().team.0, "poorer-free");
    }

    #[test]
    fn no_credited_workspace_at_all_is_refused() {
        assert!(choose(&[], &BTreeSet::new()).is_none());
    }
}
