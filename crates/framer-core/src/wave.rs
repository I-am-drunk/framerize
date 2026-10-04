//! Environment-level wave guard: the mechanism-aware response to Castle
//! risk-refusal waves, grounded in Castle's own documentation.
//!
//! What the docs establish (castle-docs, 2026-10-04):
//!  - `high_activity_account/device/ip` signals fire on "anomalous bursts of
//!    requests" RELATIVE TO A LEARNED BASELINE. Sustained bursts push the bot
//!    score over the deny threshold; it decays within minutes of quiet.
//!  - The failed/success event RATIO feeds the model: every refused request
//!    makes the next refusal more likely. Retrying into a wave is the
//!    feedback loop that turns a 2-minute flag into a 15-minute outage.
//!  - Verdicts are per-request, real-time, and IP-sensitive — a wave is an
//!    ENVIRONMENT condition (one egress IP, all accounts), not an account
//!    condition. Per-account cooldowns are the wrong abstraction mid-wave.
//!
//! So: when refusals CLUSTER ACROSS ACCOUNTS, stop ALL traffic briefly. No
//! probes, no retries, nothing for the ratio to feed on. This is the whole
//! fix — the score decays on quiet by design.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How many refusals on DISTINCT accounts inside `WAVE_WINDOW` mean "the
/// environment is being denied", not "one account is in trouble".
const WAVE_MIN_ACCOUNTS: usize = 2;
const WAVE_WINDOW: Duration = Duration::from_secs(90);
/// First pause. Each consecutive wave before a clean `QUIET_RESET` doubles it.
const PAUSE_BASE: Duration = Duration::from_secs(150);
const PAUSE_MAX: Duration = Duration::from_secs(900);
/// This much quiet (no refusals) resets the escalation ladder.
const QUIET_RESET: Duration = Duration::from_secs(600);

pub struct WaveGuard {
    inner: Mutex<Inner>,
}

struct Inner {
    /// (account-id, when) for recent risk-refusals.
    refusals: VecDeque<(String, Instant)>,
    paused_until: Option<Instant>,
    pause_len: Duration,
    last_refusal: Option<Instant>,
}

impl Default for WaveGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl WaveGuard {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                refusals: VecDeque::new(),
                paused_until: None,
                pause_len: PAUSE_BASE,
                last_refusal: None,
            }),
        }
    }

    /// Call before spending a mint+request. Returns the remaining pause if
    /// the environment is in a wave cool-off (fail fast; do not feed it).
    pub fn check(&self) -> Option<Duration> {
        let inner = self.inner.lock().ok()?;
        let until = inner.paused_until?;
        let now = Instant::now();
        (until > now).then(|| until.duration_since(now))
    }

    /// Record one risk-refusal for an account. Triggers the wave pause when
    /// refusals span WAVE_MIN_ACCOUNTS distinct accounts inside the window.
    pub fn note_refusal(&self, account_id: &str) {
        let Ok(mut inner) = self.inner.lock() else { return };
        let now = Instant::now();
        let prev_refusal = inner.last_refusal;
        inner.last_refusal = Some(now);
        while inner
            .refusals
            .front()
            .map(|(_, t)| now.duration_since(*t) > WAVE_WINDOW)
            .unwrap_or(false)
        {
            inner.refusals.pop_front();
        }
        inner.refusals.push_back((account_id.to_string(), now));
        let distinct: std::collections::HashSet<&String> =
            inner.refusals.iter().map(|(a, _)| a).collect();
        let distinct_len = distinct.len();
        drop(distinct);
        if distinct_len >= WAVE_MIN_ACCOUNTS {
            // Escalate if waves keep happening; the ladder resets on quiet.
            if let Some(prev) = prev_refusal {
                if now.duration_since(prev) > QUIET_RESET {
                    inner.pause_len = PAUSE_BASE;
                }
            }
            let len = inner.pause_len.min(PAUSE_MAX);
            inner.paused_until = Some(now + len);
            inner.pause_len = (inner.pause_len * 2).min(PAUSE_MAX);
            eprintln!(
                "[wave] risk-refusals across {} accounts inside {:?} — pausing ALL traffic {:?} (ratio feed must stop)",
                distinct_len,
                WAVE_WINDOW,
                len
            );
            inner.refusals.clear();
        }
    }

    /// A success means the environment is serving again: clear any pause.
    pub fn note_success(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.paused_until = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wave_pauses_on_cross_account_cluster() {
        let g = WaveGuard::new();
        assert!(g.check().is_none());
        g.note_refusal("a");
        assert!(g.check().is_none()); // one account is not a wave
        g.note_refusal("b");
        assert!(g.check().is_some()); // two accounts inside 90s: wave
        g.note_success();
        assert!(g.check().is_none());
    }
}
