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

/// The billing floor for "this workspace can serve". Default 2.0 (env
/// FRAMERIZE_CREDIT_FLOOR overrides): sized from measured settles, not fear.
/// A streamed leg settles ~0.1 credits (custom-chain ledger deltas); the
/// worst settle ever observed was 1.18 (the raced sol leg, 2026-11-09). The
/// old default of 25 dead-marked every workspace under 25 credits -- with
/// per-request costs at ~0.1 that stranded ~250 requests per workspace and
/// skipped whole accounts over it (shopping_footgear977 sat dead with 4.4
/// credits it could have served ~40 requests from). 2.0 covers even the
/// worst single settle with room; a workspace that dips below it mid-chain
/// 402s once and fails over, which is exactly what the failover is for.
pub const DEFAULT_CREDIT_FLOOR: f64 = 2.0;

// GLOBAL SPEND GATE (2026-11-10): zero-spend is the DEFAULT on this daemon.
// Every billable upstream open must pass spend_allowed() BEFORE it fires.
// An operator opts into spend explicitly with FRAMERIZE_ALLOW_SPEND=1;
// otherwise ONLY the zero-proved models pass and everything else is refused
// loud with SpendDenied (never silent, never billed). This is the second
// wall behind each route's own allowlist (defense in depth): even a future
// route that forgets its own gate cannot spend by accident.
pub const CUTOFF_FREE_MODELS: &[&str] = &["gpt-4.1-mini", "claude-haiku-4.5", "gpt-6-astra"];

/// True when the daemon may open a billable upstream call for this model.
pub fn spend_allowed(model: &str) -> bool {
    if std::env::var("FRAMERIZE_ALLOW_SPEND").map(|v| v == "1").unwrap_or(false) {
        return true;
    }
    let slug = model.rsplit('/').next().unwrap_or(model);
    let slug = slug.trim();
    if slug.is_empty() {
        return true; // empty resolves to the free mini downstream
    }
    CUTOFF_FREE_MODELS.contains(&slug)
}

/// Bridge-level rule (2026-11-10): Some(reason) when an upstream POST to
/// `path` carrying `model` must be REFUSED before it fires. Free paths
/// (auth, users/me, credits, refund) always pass (None); inference routes
/// pass only via spend_allowed(). The three bridge/stream mouths share this
/// so the rule cannot drift between them.
pub fn spend_gate_check(path: &str, model: &str) -> Option<String> {
    let billable = path.starts_with("/ai/v3/chat")
        || path.starts_with("/ai/v3/messages")
        || path.starts_with("/ai/v3/responses")
        || path.starts_with("/ai/v3/openai/");
    if billable && !spend_allowed(model) {
        return Some(spend_denied_reason(model));
    }
    None
}

/// Loud refusal reason (mirrors the cutoff allowlist message).
pub fn spend_denied_reason(model: &str) -> String {
    format!(
        "spend denied for model {model}: zero-spend default is ON (set FRAMERIZE_ALLOW_SPEND=1 to allow spend). Free models: gpt-4.1-mini, claude-haiku-4.5. No upstream call was opened, nothing spent."
    )
}

pub fn bill_floor() -> f64 {
    std::env::var("FRAMERIZE_CREDIT_FLOOR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CREDIT_FLOOR)
}

/// True when the account MIGHT bill somewhere: at least one workspace is not
/// KNOWN dead. Known-dead means a fresh ledger read below the floor; unknown
/// balances stay eligible (fail-open -- the per-request funded-scope picker
/// is the real guard, and the first read populates the cache). An account
/// whose EVERY workspace is proven empty can only 402 at dispatch; the
/// selector must not land clients on it.
pub fn account_might_bill(a: &ManagedAccount) -> bool {
    let floor = bill_floor();
    let scopes = a.all_scopes();
    if scopes.is_empty() {
        return false;
    }
    scopes.iter().any(|sc| {
        if is_dead(&sc.team.0) {
            return false;
        }
        match cached_live_balance(&sc.team.0) {
            Some(b) => b >= floor,
            None => true,
        }
    })
}

// ---- Live workspace ledger cache (2026-11-09) ----
// The credits endpoint is rate-limited and the ledger is the ground truth for
// "can this workspace bill". Reading it per request per workspace is the only
// way to never route onto an empty workspace (the 402 at dispatch). Cached
// 60s: the meter settles per request, and a 60s-old read is still a thousand
// times fresher than the per-account snapshot (which went stale by 500
// credits and caused the credits-exhausted chains).
static LIVE_BAL: Mutex<Option<HashMap<String, (f64, u64)>>> = Mutex::new(None);
/// Fresh-read horizon: balance reads older than this are re-fetched.
pub const LIVE_BAL_TTL_MS: u64 = 60_000;

/// The cached live balance for a workspace, only when fresh.
pub fn cached_live_balance(team: &str) -> Option<f64> {
    let now = now_ms();
    LIVE_BAL
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(team).copied())
        .filter(|(_, at)| now.saturating_sub(*at) < LIVE_BAL_TTL_MS)
        .map(|(b, _)| b)
}

/// Record a fresh live balance for a workspace (any reader: preflight reads,
/// usage-header observations, the account view). A below-floor read ALSO
/// dead-marks the workspace for DEAD_TTL_MS: dead workspaces are skipped
/// without a fresh read (fast next time), and the mark clears the moment a
/// positive balance arrives (a credited workspace returns by itself).
/// Renewal dates per team (ISO strings from funds[].renewsAt): a drained
/// workspace is a date, not a death. Fed by the same live reads that feed
/// the balance cache; surfaced in the accounts view.
static RENEWALS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

pub fn note_live_renewal(team: &str, renews_at: Option<String>) {
    if let Some(date) = renews_at {
        if let Ok(mut g) = RENEWALS.lock() {
            g.get_or_insert_with(HashMap::new).insert(team.to_string(), date);
        }
    }
}

pub fn cached_renewal(team: &str) -> Option<String> {
    RENEWALS
        .lock()
        .ok()
        .and_then(|g| g.as_ref()?.get(team).cloned())
}

pub fn note_live_balance(team: &str, balance: f64) {
    let mut g = LIVE_BAL.lock().unwrap();
    g.get_or_insert_with(HashMap::new)
        .insert(team.to_string(), (balance, now_ms()));
    let mut d = DEAD.lock().unwrap();
    let m = d.get_or_insert_with(HashMap::new);
    if balance < bill_floor() {
        m.insert(team.to_string(), now_ms() + DEAD_TTL_MS);
    } else {
        m.remove(team);
    }
}

/// Gated/blocked models: model slug -> gated-until (unix ms). A plan-gated
/// or guardrail-blocked model's failure used to burn device velocity (every
/// retry minted + posted), cooling accounts for 600s and 503ing everything
/// else -- the recurring "overloaded" storm. Learn it once, then fail fast at
/// admission with zero burn; the mark expires and re-probes later.
static GATED: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);
/// Gated-model marks live 15 min: Framer's plan gate and OpenRouter's
/// guardrails change on that scale; short enough to re-probe, long enough to
/// stop a retry storm.
pub const GATED_TTL_MS: u64 = 900_000;

/// Normalize any spelling to the gate key: strip provider prefix and any
/// :suffix (":free", ":online", ...) so "anthropic/claude-opus-5.5" and
/// "claude-opus-5.5:free" share one entry.
pub fn model_gate_key(model: &str) -> String {
    let slug = model.rsplit('/').next().unwrap_or(model);
    slug.split(':').next().unwrap_or(slug).to_string()
}

/// Mark a model gated (plan gate or guardrail block).
pub fn mark_model_gated(model: &str) {
    let key = model_gate_key(model);
    let mut g = GATED.lock().unwrap();
    g.get_or_insert_with(HashMap::new).insert(key, now_ms() + GATED_TTL_MS);
}

/// Clear the mark (a model that just worked must serve).
pub fn note_model_ok(model: &str) {
    let key = model_gate_key(model);
    if let Some(m) = GATED.lock().unwrap().as_mut() {
        m.remove(&key);
    }
}

/// True while the model is gated-marked.
pub fn is_model_gated(model: &str) -> bool {
    let key = model_gate_key(model);
    GATED.lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(&key).copied())
        .map(|u| u > now_ms())
        .unwrap_or(false)
}

/// Dead-workspace marks: workspace -> dead-until (unix ms). Dead workspaces
/// are the ones that 402 at dispatch; marking them makes selection instant.
static DEAD: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);
/// A dead workspace stays marked for 5 minutes (they refill on minutes, not
/// seconds); the 60s balance cache alone would re-admit them every minute.
pub const DEAD_TTL_MS: u64 = 300_000;

/// True when the workspace is currently dead-marked.
pub fn is_dead(team: &str) -> bool {
    DEAD.lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(team).copied())
        .map(|u| u > now_ms())
        .unwrap_or(false)
}

/// Force-refresh marker for tests: drop a workspace's cached read.
#[cfg(test)]
pub fn clear_live_balance_cache() {
    *LIVE_BAL.lock().unwrap() = None;
    *DEAD.lock().unwrap() = None;
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
    // Exclude accounts whose every workspace is KNOWN empty (fresh ledger
    // read below the floor): they can only 402 at dispatch. Unknown-balance
    // accounts stay eligible. If EVERYTHING is known-dead, keep the set (the
    // loud per-account error beats a silent none).
    let billable: Vec<&ManagedAccount> = available
        .iter()
        .filter(|a| account_might_bill(a))
        .cloned()
        .collect();
    // Prove the exclusion in the daemon log: a known-dead account dropping
    // out of the candidate set is exactly the behavior the operator asked
    // for ("exclude it from the pool count").
    for a in &available {
        if !account_might_bill(a) {
            eprintln!(
                "[rotation] excluding account {} (all workspaces known-empty)",
                &a.id.0.to_string()[..8.min(a.id.0.to_string().len())]
            );
        }
    }
    let available = if billable.is_empty() { available } else { billable };

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
    // Tie-break on RAW failures before the round-robin cursor: effective_failures
    // forgives a served cooldown (used in is_available_at already), so without
    // this a post-cooldown account with unresolved upstream credit risk (0 eff /
    // N raw) ties a never-failed funded account (0 eff / 0 raw) and the cursor
    // can lease the dead one. Never-failed wins; the cursor still rotates
    // among the cleanest. Never starves: this only reorders the tied set.
    let best_raw = candidates
        .iter()
        .filter(|a| a.effective_failures(now) == best)
        .map(|a| a.consecutive_failures)
        .min()?;
    let tied: Vec<&ManagedAccount> = candidates
        .into_iter()
        .filter(|a| a.effective_failures(now) == best && a.consecutive_failures == best_raw)
        .collect();

    // `tied` is non-empty: `best` came from the same filtered set.
    let n = tied.len();
    let i = CURSOR.fetch_add(1, Ordering::Relaxed) % n;
    Some(tied[i])
}

#[cfg(test)]
mod tests {
    static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn spend_gate_check_covers_all_mouths() {
        std::env::remove_var("FRAMERIZE_ALLOW_SPEND");
        // Free paths always pass, even with denied models.
        assert!(spend_gate_check("/auth/v4/signin", "kimi-k3").is_none());
        assert!(spend_gate_check("/web/users/me", "anything").is_none());
        assert!(spend_gate_check("/web/teams/t/ai-credits", "kimi-k3").is_none());
        assert!(spend_gate_check("/ai/v3/refund/", "kimi-k3").is_none());
        // Inference routes: allowlisted pass, others refused.
        assert!(spend_gate_check("/ai/v3/chat/completions", "gpt-4.1-mini").is_none());
        assert!(spend_gate_check("/ai/v3/messages/", "claude-haiku-4.5").is_none());
        assert!(spend_gate_check("/ai/v3/responses/", "gpt-6-astra").is_none());
        assert!(spend_gate_check("/ai/v3/openai/responses/", "gpt-4.1-mini").is_none());
        assert!(spend_gate_check("/ai/v3/chat/completions", "kimi-k3").is_some());
        assert!(spend_gate_check("/ai/v3/chat/completions", "gpt-4.1").is_some());
        assert!(spend_gate_check("/ai/v3/messages/", "gemini-2.5-flash").is_some());
        assert!(spend_gate_check("/ai/v3/openai/responses/", "gpt-6-sol").is_some());
    }

    #[test]
    fn spend_gate_allows_only_free_models_by_default() {
        let _g = ENV_GUARD.lock().unwrap();
        std::env::remove_var("FRAMERIZE_ALLOW_SPEND");
        assert!(spend_allowed("gpt-4.1-mini"));
        assert!(spend_allowed("claude-haiku-4.5"));
        assert!(spend_allowed("openai/gpt-4.1-mini"));
        assert!(spend_allowed(""));
        assert!(!spend_allowed("kimi-k3"));
        assert!(!spend_allowed("gpt-4.1"));
        assert!(!spend_allowed("gemini-2.5-flash"));
        assert!(!spend_allowed("gpt-6-sol"));
        assert!(spend_denied_reason("kimi-k3").contains("nothing spent"));
    }

    #[test]
    fn spend_gate_opt_in_allows_all() {
        let _g = ENV_GUARD.lock().unwrap();
        std::env::set_var("FRAMERIZE_ALLOW_SPEND", "1");
        assert!(spend_allowed("kimi-k3"));
        assert!(spend_allowed("anything-at-all"));
        std::env::remove_var("FRAMERIZE_ALLOW_SPEND");
        assert!(!spend_allowed("kimi-k3"));
    }

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
            extra_workspaces: Vec::new(),
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
    fn known_dead_account_is_excluded_when_a_funded_one_exists() {
        clear_live_balance_cache();
        let dead = acct("dead", 0, AccountState::Ready);
        // Every scope of `dead` is known-empty: the fixture's own team AND
        // its extra workspace both read 0.
        note_live_balance("team", 0.0);
        let mut healthy = acct("healthy", 0, AccountState::Ready);
        // Own unknown-balance team: stays eligible (fail-open for unknowns).
        healthy.current_team = Some(TeamId("goodteam".into()));
        let pool = vec![dead, healthy];
        for _ in 0..4 {
            assert_eq!(pick(&pool).expect("a pick").alias, "healthy");
        }
    }

    #[test]
    fn unknown_balance_account_stays_eligible() {
        clear_live_balance_cache();
        let a = acct("unknown", 0, AccountState::Ready);
        let b = acct("other", 0, AccountState::Ready);
        let pool = vec![a, b];
        let picks: Vec<&str> = (0..4).map(|_| pick(&pool).unwrap().alias.as_str()).collect();
        assert!(picks.contains(&"unknown"), "unknown must stay eligible: {picks:?}");
    }

    #[test]
    fn all_known_dead_still_picks_loudly() {
        clear_live_balance_cache();
        let a = acct("dead-a", 0, AccountState::Ready);
        note_live_balance("team", 0.0);
        let b = acct("dead-b", 0, AccountState::Ready);
        let pool = vec![a, b];
        assert!(pick(&pool).is_some(), "all-dead must still pick (loud failure beats none)");
    }

    #[test]
    fn dead_marked_workspace_skips_until_revived_by_a_positive_read() {
        clear_live_balance_cache();
        let mut dead = acct("dead", 0, AccountState::Ready);
        dead.current_team = Some(TeamId("deadteam".into()));
        note_live_balance("deadteam", 0.0); // dead-marks it
        let mut healthy = acct("healthy", 0, AccountState::Ready);
        healthy.current_team = Some(TeamId("goodteam".into()));
        let pool = vec![dead.clone(), healthy.clone()];
        for _ in 0..4 {
            assert_eq!(pick(&pool).expect("a pick").alias, "healthy");
        }
        // A positive read clears the dead mark: the account returns.
        note_live_balance("deadteam", 400.0);
        assert!(account_might_bill(&dead));
        let picks: Vec<&str> = (0..4).map(|_| pick(&pool).unwrap().alias.as_str()).collect();
        assert!(picks.contains(&"dead"), "revived account must return: {picks:?}");
    }

    #[test]
    fn empty_pool_yields_none() {
        assert!(pick(&[]).is_none());
        let all_out = vec![acct("cooling", 0, AccountState::CoolingDown)];
        assert!(pick(&all_out).is_none());
    }
}
