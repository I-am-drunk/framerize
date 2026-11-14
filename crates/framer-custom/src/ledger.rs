//! Credits-ledger oracle helpers: per-leg zero-settlement assert + policy.
//!
//! Proven behavior: after every leg the engine reads the ledger, and a
//! nonzero debit fails the leg. is_settled is that comparison; after_leg is
//! the pure disposition the engine calls with the rung outcome + the debit
//! flag. Retry budgeting and the actual quarantine live in the engine.

/// Ledger epsilon: balance wobble at or under this counts as zero.
pub const LEDGER_EPS: f64 = 1e-4;

// SPEND CIRCUIT BREAKER (2026-11-10): latched the moment any cutoff chain
// reports a nonzero settle. While latched, cutoff_run refuses every request
// BEFORE any upstream open (fail-closed). Cleared only by an explicit
// operator reset (or daemon restart -- it is intentionally NOT self-healing:
// a changed meter must be re-proved, not assumed back).
static CUT_SPENT: std::sync::Mutex<Option<u64>> = std::sync::Mutex::new(None);

/// Disk latch path: the in-process latch dies on restart (deploy/crash), silently
/// un-halting a changed meter. Trip persists a file; tripped() honors it; reset removes
/// it. Explicit path wins, else $HOME/.framerize.
fn latch_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("FRAMERIZE_BREAKER_FILE") {
        if !p.is_empty() {
            return Some(std::path::PathBuf::from(p));
        }
    }
    std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".framerize/spend_breaker.latched"))
}
fn latch_write() {
    if let Some(p) = latch_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&p, now_ms().to_string());
    }
}
fn latch_clear() {
    if let Some(p) = latch_path() {
        let _ = std::fs::remove_file(&p);
    }
}
fn latch_present() -> bool {
    latch_path().map(|p| p.exists()).unwrap_or(false)
}

/// Floor: only settles bigger than this trip the GLOBAL breaker. Dust (<=0.05,
/// confirmed-persistent meter behavior on short legs, live-measured) quarantines the
/// REQUEST (withhold + 402) but must not halt ALL serving -- tripping on dust wedged
/// the system on ghosts (5 trips in 20 min, pool barely moving). Large settles signal
/// a changed meter or a billed path and halt everything until an operator re-proves.
pub const BREAKER_TRIP_FLOOR: f64 = 0.05;
/// Aggregate floor: trip the global breaker when the *sum* of sub-floor debits
/// exceeds this, too. The old per-debit-only floor let N tiny dust settles
/// (and a large confirmed settle) bleed silently -- the live -0.936 unmetered
/// drain never tripped it. Aggregate catches the total, not just the largest.
pub const AGGREGATE_TRIP_FLOOR: f64 = 0.20;
static AGGREGATE_DEBIT_UX: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// Latch the breaker (called with the observed debit for the log line).
pub fn trip_spend_breaker(debit: f64) {
    use std::sync::atomic::Ordering;
    let mag = debit.abs();
    if mag > 0.0 {
        let ux = (mag * 1e6) as i64;
        let prev = AGGREGATE_DEBIT_UX.fetch_add(ux, Ordering::SeqCst);
        let agg = (prev + ux) as f64 / 1e6;
        if mag <= BREAKER_TRIP_FLOOR && agg <= AGGREGATE_TRIP_FLOOR {
            eprintln!("[spend-breaker] dust {debit:+.6} (under floor; agg {agg:.6} <= {AGGREGATE_TRIP_FLOOR}) -- quarantine only, breaker NOT tripped");
            return;
        }
        eprintln!("[spend-breaker] debit {debit:+.6} agg {agg:.6} >= floor -> TRIP");
    }
    if let Ok(mut g) = CUT_SPENT.lock() {
        if g.is_none() {
            *g = Some(now_ms());
            latch_write();
            eprintln!("[spend-breaker] TRIPPED on debit {debit:+.6} -- cutoff legs halted until reset");
        }
    }
}

/// True while the breaker is latched (memory OR disk: restarts must not silently
/// un-halt a changed meter).
pub fn spend_breaker_tripped() -> bool {
    if CUT_SPENT.lock().map(|g| g.is_some()).unwrap_or(false) {
        return true;
    }
    if latch_present() {
        // Re-latch memory so the log line below fires once per process, not per check.
        if let Ok(mut g) = CUT_SPENT.lock() {
            if g.is_none() {
                *g = Some(now_ms());
                eprintln!("[spend-breaker] latched at startup from disk -- cutoff legs halted until operator reset");
            }
        }
        return true;
    }
    false
}

/// Operator reset (wired to the status route's POST reset).
pub fn reset_spend_breaker() {
    use std::sync::atomic::Ordering;
    if let Ok(mut g) = CUT_SPENT.lock() {
        *g = None;
    }
    AGGREGATE_DEBIT_UX.store(0, Ordering::SeqCst);
    latch_clear();
    eprintln!("[spend-breaker] RESET by operator");
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// True when the ledger did not move: |after - before| <= EPS.
/// Non-finite inputs (NaN/+inf from a broken oracle read) NEVER count as
/// settled: a poisoned oracle fails legs instead of passing them.
pub fn is_settled(before: f64, after: f64) -> bool {
    if !before.is_finite() || !after.is_finite() {
        return false;
    }
    (after - before).abs() <= LEDGER_EPS
}

/// Signed ledger move (after - before) for debit accounting. Non-finite
/// inputs yield 0.0 so accounting never propagates NaN into totals; the
/// leg still counts as debited via is_settled above.
pub fn signed_delta(before: f64, after: f64) -> f64 {
    if !before.is_finite() || !after.is_finite() {
        return 0.0;
    }
    after - before
}

/// Engine disposition for a finished leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// The leg stands as captured.
    Accept,
    /// The leg may have settled or echoed: retry it once with a halved stop
    /// (see stop halt_after_race); the engine quarantines once its retry
    /// budget is spent.
    RetryHalved,
    /// The leg is tainted: set it aside and surface it, never silently keep
    /// it. Reached by the engine once retries are exhausted -- not by
    /// after_leg: the four first-look outcomes below only Accept or schedule
    /// one halved retry.
    QuarantineAndReport,
}

/// First-look policy from the rung outcome + whether the ledger moved:
/// raced without debit -> Accept; raced with debit, or no race with debit
/// -> RetryHalved; no race without debit (the clean abort) -> Accept.
pub fn after_leg(raced: bool, debited: bool) -> Policy {
    match (raced, debited) {
        (true, false) => Policy::Accept,
        (false, false) => Policy::Accept,
        _ => Policy::RetryHalved,
    }
}

    #[test]
    fn breaker_survives_process_restart_via_disk() {
        // Simulate trip -> drop memory latch (restart) -> still tripped via disk.
        let dir = std::env::temp_dir().join(format!("breaker-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("latched");
        std::env::set_var("FRAMERIZE_BREAKER_FILE", &f);
        reset_spend_breaker();
        assert!(!spend_breaker_tripped());
        trip_spend_breaker(-1.0);
        assert!(spend_breaker_tripped());
        // "Restart": clear memory only (disk file stays).
        CUT_SPENT.lock().unwrap().take();
        assert!(spend_breaker_tripped(), "disk latch must re-trip after restart");
        reset_spend_breaker();
        assert!(!spend_breaker_tripped());
        assert!(!f.exists(), "reset removes disk latch");
        std::env::remove_var("FRAMERIZE_BREAKER_FILE");
        let _ = std::fs::remove_dir_all(&dir);
    }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{CustomTransport, MockTransport};

    #[test]
    fn zero_assert_with_eps() {
        assert!(is_settled(100.0, 100.0));
        assert!(!is_settled(100.0, 100.5));
        assert!(!is_settled(100.0, 99.5));
        assert!(is_settled(0.0, LEDGER_EPS));
        assert!(is_settled(0.0, -LEDGER_EPS));
        assert!(!is_settled(0.0, 2.0 * LEDGER_EPS));
        assert!(!is_settled(100.0, 100.0 + LEDGER_EPS * 1.5));
    }

    #[test]
    fn nonfinite_never_settles() {
        assert!(!is_settled(f64::NAN, 100.0));
        assert!(!is_settled(100.0, f64::NAN));
        assert!(!is_settled(f64::NAN, f64::NAN));
        assert!(!is_settled(f64::INFINITY, f64::INFINITY));
        assert!(!is_settled(100.0, f64::INFINITY));
        assert_eq!(signed_delta(f64::NAN, 100.0), 0.0);
        assert_eq!(signed_delta(100.0, f64::INFINITY), 0.0);
    }

    #[test]
    fn signed_delta_direction() {
        assert_eq!(signed_delta(100.0, 99.5), -0.5);
        assert_eq!(signed_delta(100.0, 100.0), 0.0);
    }

    #[test]
    fn policy_table() {
        use Policy::*;
        assert_eq!(after_leg(true, false), Accept);
        assert_eq!(after_leg(false, false), Accept);
        assert_eq!(after_leg(true, true), RetryHalved);
        assert_eq!(after_leg(false, true), RetryHalved);
    }

    #[tokio::test]
    async fn oracle_reads_through() {
        let t = MockTransport::new(vec![]).with_credits(42.5);
        assert_eq!(t.read_credits().await.unwrap(), 42.5);
    }
}
