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
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Round-robin cursor, advanced once per tie-broken pick.
static CURSOR: AtomicUsize = AtomicUsize::new(0);

/// Workspace (team) -> saturated-until (unix ms). A 429 means "this workspace
/// is full right now" — the account is FINE, so this never touches health
/// counters; it only steers the next picks to other workspaces for ~45s.
/// Without it (measured 2026-10-05 15:25): a burst saturated one workspace,
/// and health-blind round-robin kept landing client retries on it — every
/// attempt 429'd, clients gave up ("repeated API errors").
static SATURATED: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

pub fn mark_saturated(team: &str, backoff_ms: u64) {
    let until = now_ms() + backoff_ms;
    let mut g = SATURATED.lock().unwrap();
    g.get_or_insert_with(HashMap::new)
        .insert(team.to_string(), until);
    eprintln!("[rotation] workspace {team} saturated for {}s", backoff_ms / 1000);
}

fn saturated_until(team: &str) -> Option<u64> {
    SATURATED
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(team).copied())
}

/// Per-account round-robin index over its workspace scopes (the concurrency
/// goose: chains spread across every workspace the account owns).
static SCOPE_RR: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);

pub fn next_scope_idx(account_id: &str, n_scopes: usize) -> usize {
    if n_scopes == 0 {
        return 0;
    }
    let mut g = SCOPE_RR.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    let i = m.entry(account_id.to_string()).or_insert(0);
    let cur = *i % n_scopes;
    *i = i.wrapping_add(1);
    cur
}

/// True if the workspace is currently marked saturated (used by the scope
/// picker to prefer non-saturated scopes without a full pick cycle).
pub fn is_saturated(team: &str) -> bool {
    saturated_until(team).map(|u| u > now_ms()).unwrap_or(false)
}

/// Pick the account to serve the next request, or `None` if the pool is dry.
///
/// Health first: an account with fewer consecutive failures always wins, so a
/// cooling-down account is still avoided. Among the healthiest, requests are
/// handed out round-robin.
pub fn pick(pool: &[ManagedAccount]) -> Option<&ManagedAccount> {
    pick_at(pool, now_ms())
}

/// Wall clock in unix ms. Separate so tests can pin it.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `pick` with an injected clock.
///
/// Cooling-down accounts are skipped entirely. If EVERY account is cooling we
/// fall back to the one whose cooldown expires soonest rather than returning
/// None: a transient refusal must never take the whole gateway down, and the
/// caller's own retry/backoff handles the rest.
pub fn pick_at(pool: &[ManagedAccount], now: u64) -> Option<&ManagedAccount> {
    let available: Vec<&ManagedAccount> =
        pool.iter().filter(|a| a.is_available_at(now)).collect();

    // Workspace-saturation steering: skip accounts whose team is currently
    // 429-saturated while any alternative exists. All-saturated falls back to
    // the least-soon-saturated set (a transient must never dry the pool).
    let unsaturated: Vec<&ManagedAccount> = available
        .iter()
        .filter(|a| {
            a.current_team
                .as_ref()
                .map(|t| saturated_until(&t.0).map(|u| u <= now).unwrap_or(true))
                .unwrap_or(true)
        })
        .cloned()
        .collect();
    let effective: Vec<&ManagedAccount> = if !unsaturated.is_empty() {
        unsaturated
    } else if !available.is_empty() {
        available.clone()
    } else {
        available
    };

    let candidates: Vec<&ManagedAccount> = if effective.is_empty() {
        let soonest = pool
            .iter()
            .filter(|a| a.is_eligible())
            .min_by_key(|a| a.cooling_until_ms.unwrap_or(0))?;
        vec![soonest]
    } else {
        effective
    };

    // Rank by EFFECTIVE failures, not the raw count. An account that has served
    // its cooldown window has already paid for those failures; keeping them in
    // the ranking starves it forever (failures >= 1 vs a healthy sibling's 0 =>
    // never picked => count never clears). Accounts failing WITHOUT a cooldown
    // -- transport errors, say -- are still deprioritized.
    let best = candidates.iter().map(|a| a.effective_failures(now)).min()?;
    let tied: Vec<&ManagedAccount> = candidates
        .into_iter()
        .filter(|a| a.effective_failures(now) == best)
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
            email: Some(format!("{alias}@example.test")),
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
            cooling_until_ms: None,
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
    fn a_cooling_account_is_skipped_until_its_window_expires() {
        // The live 2026-10-04 failure: one account went 100% 403 for 69
        // consecutive posts because nothing ever took it out of rotation.
        let mut poisoned = acct("poisoned", 0, AccountState::Ready);
        poisoned.note_risk_refused(1_000);
        let healthy = acct("healthy", 0, AccountState::Ready);
        let pool = vec![poisoned, healthy];

        for _ in 0..6 {
            assert_eq!(pick_at(&pool, 1_500).expect("a pick").alias, "healthy");
        }
        // After the backoff window it is usable again -- never permanently out.
        let after = 1_000 + ManagedAccount::cooldown_backoff_ms(0) + 1;
        let seen: Vec<&str> = (0..6).map(|_| pick_at(&pool, after).unwrap().alias.as_str()).collect();
        assert!(seen.contains(&"poisoned"), "must return to rotation: {seen:?}");
    }

    #[test]
    fn when_everything_is_cooling_the_soonest_to_recover_is_used() {
        // A transient refusal must never take the whole gateway down.
        let mut a = acct("a", 0, AccountState::Ready);
        let mut b = acct("b", 0, AccountState::Ready);
        a.note_risk_refused(10_000); // longer window
        b.cooling_until_ms = Some(10_500);
        let pool = vec![a, b];
        assert_eq!(pick_at(&pool, 10_100).expect("a pick").alias, "b");
    }

    #[test]
    fn backoff_grows_then_caps_and_success_clears_it() {
        assert_eq!(ManagedAccount::cooldown_backoff_ms(0), 2_000);
        assert_eq!(ManagedAccount::cooldown_backoff_ms(1), 4_000);
        assert_eq!(ManagedAccount::cooldown_backoff_ms(2), 8_000);
        assert_eq!(ManagedAccount::cooldown_backoff_ms(99), 300_000, "must cap");

        let mut a = acct("a", 0, AccountState::Ready);
        a.note_risk_refused(0);
        a.note_risk_refused(0);
        assert_eq!(a.cooldown_step, 2);
        assert!(a.cooling_until_ms.is_some());
        assert!(!a.is_available_at(1));

        a.note_success();
        assert_eq!(a.cooldown_step, 0);
        assert_eq!(a.consecutive_failures, 0);
        assert!(a.cooling_until_ms.is_none());
        assert!(a.is_available_at(1));
    }

    #[test]
    fn empty_pool_yields_none() {
        assert!(pick(&[]).is_none());
        let all_out = vec![acct("cooling", 0, AccountState::CoolingDown)];
        assert!(pick(&all_out).is_none());
    }
}
