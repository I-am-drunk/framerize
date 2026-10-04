//! Account selection across the pool. Least-failed wins; ties rotate.
//!
//! Why ties must rotate: `min_by_key` is stable, so it returns the FIRST
//! element of a tie — and in the steady state every healthy account has
//! `consecutive_failures == 0`, i.e. the whole pool is one big tie. That sent
//! 2,232 of 2,232 chains on 2026-10-03 to a single account while a second
//! ready account served zero, capping concurrency at one account's page pool
//! (~3 pages) and collapsing throughput from ~85 tok/s to 3-6 tok/s under load.
//!
//! An account is a device with its own browser context and its own page pool
//! (see AGENTS.md: "Accounts are devices"), so spreading across accounts is how
//! concurrent capacity grows. Credits live on the team, so rotating between
//! accounts on the same team costs nothing.

use crate::account::ManagedAccount;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Round-robin cursor, advanced once per tie-broken pick.
static CURSOR: AtomicUsize = AtomicUsize::new(0);

/// Pick the account to serve the next request, or `None` if the pool is dry.
///
/// Health first: an account with fewer consecutive failures always wins, so a
/// cooling-down account is still avoided. Among the healthiest, requests are
/// handed out round-robin.
pub fn pick(pool: &[ManagedAccount]) -> Option<&ManagedAccount> {
    let best = pool
        .iter()
        .filter(|a| a.is_eligible())
        .map(|a| a.consecutive_failures)
        .min()?;

    let tied: Vec<&ManagedAccount> = pool
        .iter()
        .filter(|a| a.is_eligible() && a.consecutive_failures == best)
        .collect();

    // `tied` is non-empty: `best` came from the same filtered set.
    let n = tied.len();
    let i = CURSOR.fetch_add(1, Ordering::Relaxed) % n;
    Some(tied[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{AccountState, ManagedAccount};
    use crate::ids::{AccountId, FramerUserId, ProjectId, TeamId};
    use uuid::Uuid;

    fn acct(alias: &str, failures: u32, state: AccountState) -> ManagedAccount {
        ManagedAccount {
            id: AccountId(Uuid::new_v4()),
            alias: alias.to_string(),
            framer_user_id: FramerUserId(Uuid::new_v4().to_string()),
            castle_device_id: Uuid::new_v4().to_string(),
            proxy: None,
            // A fixture account is fully provisioned by default, because
            // is_eligible() now requires both (see account.rs).
            current_team: Some(TeamId("team".to_string())),
            current_project: Some(ProjectId("proj".to_string())),
            state,
            credit: None,
            consecutive_failures: failures,
            cooldown_step: 0,
        }
    }

    #[test]
    fn spreads_across_equally_healthy_accounts() {
        let pool = vec![
            acct("main", 0, AccountState::Ready),
            acct("probe", 0, AccountState::Ready),
        ];
        let picks: Vec<&str> = (0..4)
            .map(|_| pick(&pool).expect("a pick").alias.as_str())
            .collect();
        // Both accounts must appear — the 2026-10-03 bug was that only the
        // first ever did.
        assert!(picks.contains(&"main"), "main never picked: {picks:?}");
        assert!(picks.contains(&"probe"), "probe never picked: {picks:?}");
    }

    #[test]
    fn prefers_healthier_account_over_rotation() {
        let pool = vec![
            acct("sick", 3, AccountState::Ready),
            acct("healthy", 0, AccountState::Ready),
        ];
        for _ in 0..4 {
            assert_eq!(pick(&pool).expect("a pick").alias, "healthy");
        }
    }

    #[test]
    fn skips_ineligible_accounts() {
        let pool = vec![
            acct("cooling", 0, AccountState::CoolingDown),
            acct("ready", 2, AccountState::Ready),
        ];
        for _ in 0..3 {
            assert_eq!(pick(&pool).expect("a pick").alias, "ready");
        }
    }

    #[test]
    fn skips_accounts_missing_a_project_or_team() {
        // A newly added account sits Ready with no project until provisioning
        // finishes. Rotation must not hand it work: every request routed there
        // fails instantly with "account has no current project" (live
        // 2026-10-04, a third account took 1/3 of traffic straight to an error).
        let mut half = acct("half-provisioned", 0, AccountState::Ready);
        half.current_project = None;
        let pool = vec![half, acct("ready", 0, AccountState::Ready)];
        for _ in 0..4 {
            assert_eq!(pick(&pool).expect("a pick").alias, "ready");
        }

        let mut no_team = acct("no-team", 0, AccountState::Ready);
        no_team.current_team = None;
        assert!(pick(&[no_team]).is_none());
    }

    #[test]
    fn empty_pool_yields_none() {
        assert!(pick(&[]).is_none());
        let all_out = vec![acct("cooling", 0, AccountState::CoolingDown)];
        assert!(pick(&all_out).is_none());
    }
}
