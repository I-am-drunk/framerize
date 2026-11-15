//! Native-path ledger guard: before/after snapshots around non-cutoff chains.
//! Pure move out of lib.rs (no behavior change). On any confirmed move: log
//! LOUD with req+account, trip the spend breaker, report the debit.

use super::AppState;

/// Native-path ledger guard (2026-11-11 -- the missing wall): snapshot the scope ledger
/// before a native (non-cutoff) chain runs; re-read after. On any move: log LOUD, trip the
/// spend breaker, and report the debit. Buffered callers convert to a 402 quarantine error
/// (billed text withheld); streaming callers trip+log (frames already flowed).
pub(super) async fn native_ledger_before(
    s: &AppState,
    session: &framer_core::port::Session,
    team: &framer_core::ids::TeamId,
    req_short: &str,
    account8: &str,
) -> Option<f64> {
    match s.client.usage(session, team).await {
        Ok(snap) => Some(snap.available),
        Err(e) => {
            eprintln!("[native-guard] req={req_short} account={account8} before-read failed (gate open, serving): {e}");
            None
        }
    }
}

/// Returns Some(debit) when the ledger moved (caller withholds + trips).
pub(super) async fn native_ledger_check(
    s: &AppState,
    session: &framer_core::port::Session,
    team: &framer_core::ids::TeamId,
    before: Option<f64>,
    ctx: &str,
    req_short: &str,
    account8: &str,
) -> Option<f64> {
    let b = match before {
        Some(v) => v,
        None => return None,
    };
    match s.client.usage(session, team).await {
        Ok(snap) => {
            let d = snap.available - b;
            if d < -1e-4 {
                // HOLD-vs-SETTLE (2026-11-11): the meter places auth-HOLDS at open that
                // release seconds later (proved live: guard summed -4.89 while the pool
                // moved -1.08; balances frozen between requests). Withholding + tripping
                // on a hold wedges serving on ghosts. Confirm persistence: re-read after
                // 20s; serve normally if recovered, withhold+trip only if confirmed.
                eprintln!("[native-guard] req={req_short} account={account8} {ctx}: tentative move {d:+.6} (before {b:.2} -> after {:.2}) -- confirming in 20s", snap.available);
                tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                match s.client.usage(session, team).await {
                    Ok(snap2) => {
                        let d2 = snap2.available - b;
                        if d2 < -1e-4 {
                            eprintln!("[native-guard] req={req_short} account={account8} {ctx}: CONFIRMED settle {d2:+.6} -- withholding + tripping");
                            framer_custom::ledger::trip_spend_breaker(d2);
                            return Some(d2);
                        }
                        eprintln!("[native-guard] req={req_short} account={account8} {ctx}: hold released ({:.2} recovered) -- serving normally", snap2.available);
                        return None;
                    }
                    Err(e) => {
                        // UNKNOWN debit: steer around this team for 45s (existing
                        // backpressure pattern) instead of serving blind or wedging.
                        eprintln!("[native-guard] req={req_short} account={account8} {ctx}: confirm-read failed ({e}) -- steering around team for 45s");
                        framer_core::rotation::mark_saturated(&team.0, 45_000);
                        return None;
                    }
                }
            }
            None
        }
        Err(e) => {
            eprintln!("[native-guard] req={req_short} account={account8} {ctx}: after-read failed (cannot verify $0): {e} -- steering around team for 45s");
            framer_core::rotation::mark_saturated(&team.0, 45_000);
            None
        }
    }
}
