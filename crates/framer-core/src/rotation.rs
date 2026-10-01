//! Account selection across the pool. Kept minimal for v1: least-recently-failed,
//! eligible account wins. Grow only when a real need appears (YAGNI).

use crate::account::ManagedAccount;

/// Pick the account to serve the next request, or `None` if the pool is dry.
pub fn pick(pool: &[ManagedAccount]) -> Option<&ManagedAccount> {
    pool.iter()
        .filter(|a| a.is_eligible())
        .min_by_key(|a| a.consecutive_failures)
}
