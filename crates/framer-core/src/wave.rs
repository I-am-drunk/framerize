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
//! A cross-account cluster is retained as a process-local signal for guarded
//! account activation and lifecycle control. Inference admission must remain
//! account-scoped: a refusal cools its own account, while healthy accounts
//! stay eligible. A process-wide inference pause turns a short refusal wave
//! into a client outage and is intentionally not enforced here.

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

    /// Returns the remaining observation window used by guarded control
    /// operations. Inference admission must use each account's own cooldown.
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
        // The first refusal after quiet owns the reset. At the second refusal
        // that triggers a cluster, the previous one is at most 90 seconds old.
        if prev_refusal.is_some_and(|previous| now.duration_since(previous) > QUIET_RESET) {
            inner.pause_len = PAUSE_BASE;
        }
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
            // Escalate repeated clusters. The quiet reset already ran above.
            let len = inner.pause_len.min(PAUSE_MAX);
            inner.paused_until = Some(now + len);
            inner.pause_len = (inner.pause_len * 2).min(PAUSE_MAX);
            eprintln!(
                "[wave] risk-refusals across {} accounts inside {:?} — retaining control guard for {:?}; healthy accounts remain eligible",
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

    #[test]
    fn quiet_period_resets_escalation_before_a_new_cluster() {
        let g = WaveGuard::new();
        {
            let mut inner = g.inner.lock().unwrap();
            inner.pause_len = PAUSE_MAX;
            inner.last_refusal = Some(Instant::now() - QUIET_RESET - Duration::from_secs(1));
        }
        g.note_refusal("a");
        assert!(g.check().is_none());
        g.note_refusal("b");
        let remaining = g.check().expect("the new cluster still requires a pause");
        assert!(remaining <= PAUSE_BASE, "quiet cluster escalated to {remaining:?}");
        assert!(remaining > PAUSE_BASE - Duration::from_secs(1));
        assert_eq!(g.inner.lock().unwrap().pause_len, PAUSE_BASE * 2);
    }

    #[test]
    fn ongoing_clusters_keep_the_maximum_pause() {
        let g = WaveGuard::new();
        {
            let mut inner = g.inner.lock().unwrap();
            inner.pause_len = PAUSE_MAX;
            inner.last_refusal = Some(Instant::now());
        }
        g.note_refusal("a");
        g.note_refusal("b");
        assert!(g.check().unwrap() > PAUSE_MAX - Duration::from_secs(1));
        assert_eq!(g.inner.lock().unwrap().pause_len, PAUSE_MAX);
    }

    #[test]
    fn quiet_reset_does_not_cancel_a_current_pause() {
        let g = WaveGuard::new();
        let until = Instant::now() + PAUSE_BASE;
        {
            let mut inner = g.inner.lock().unwrap();
            inner.last_refusal = Some(Instant::now() - QUIET_RESET - Duration::from_secs(1));
            inner.paused_until = Some(until);
            inner.pause_len = PAUSE_MAX;
        }
        g.note_refusal("a");
        assert_eq!(g.inner.lock().unwrap().paused_until, Some(until));
        assert!(g.check().is_some());
    }
}

impl WaveGuard {
    /// Observe this live guard without pruning history or resetting state.
    /// A poisoned mutex is unavailable rather than clear admission.
    pub(crate) fn control_admission_status(&self) -> Result<Option<Duration>, ()> {
        let inner = self.inner.lock().map_err(|_| ())?;
        let now = Instant::now();
        Ok(inner.paused_until.filter(|until| *until > now)
            .map(|until| until.duration_since(now)))
    }
}

#[cfg(test)]
#[path = "wave_control_tests.rs"]
mod control_tests;
