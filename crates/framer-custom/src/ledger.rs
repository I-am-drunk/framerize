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

/// Latch the breaker (called with the observed debit for the log line).
pub fn trip_spend_breaker(debit: f64) {
    if let Ok(mut g) = CUT_SPENT.lock() {
        if g.is_none() {
            *g = Some(now_ms());
            eprintln!("[spend-breaker] TRIPPED on debit {debit:+.6} -- cutoff legs halted until reset");
        }
    }
}

/// True while the breaker is latched.
pub fn spend_breaker_tripped() -> bool {
    CUT_SPENT.lock().map(|g| g.is_some()).unwrap_or(false)
}

/// Operator reset (wired to the status route's POST reset).
pub fn reset_spend_breaker() {
    if let Ok(mut g) = CUT_SPENT.lock() {
        *g = None;
    }
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
