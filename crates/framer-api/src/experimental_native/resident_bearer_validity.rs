//! Pure lifetime check before admitting a retained resident bearer.
//! Inputs come from the owned auth response and its current control deadline.
//! This cannot prove token authenticity, account binding or HTTP acceptance.
use std::time::Duration;

/// Sample remaining monotonic budget before the wall clock used for `now_ms`.
/// Keep one millisecond for truncated wall-clock precision. Do not substitute
/// this arithmetic for the current owner's cancellation or context checks.
pub(crate) fn covers_resident_deadline(
    expires_at_ms: i64, now_ms: i64, remaining: Duration,
) -> bool {
    if remaining.is_zero() { return false; }
    let Some(available_ms) = expires_at_ms.checked_sub(now_ms) else { return false; };
    let Ok(available_ms) = u64::try_from(available_ms) else { return false; };
    let Some(available) = Duration::from_millis(available_ms)
        .checked_sub(Duration::from_millis(1)) else { return false; };
    available > remaining.max(Duration::from_secs(30))
}

#[cfg(test)]
#[path = "resident_bearer_validity_tests.rs"]
mod tests;
