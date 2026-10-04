//! Adaptive per-workspace concurrency gate.
//!
//! Framer enforces a server-side cap: at 100 simultaneous requests the upstream
//! returns `429 "too many concurrent AI requests for this workspace"`. Measured
//! on this box 2026-10-04: 32/32 ok in 7s, 64/64 ok in 16s, 100 -> 35/100 with
//! 65 of those 429s.
//!
//! So past ~64 the bottleneck is no longer ours. Failing the caller is the wrong
//! answer -- the work is fine, it just has to wait. This gate holds a permit per
//! in-flight request and shrinks on 429 / grows on sustained success, so a fleet
//! of 100 agents QUEUES instead of erroring.
//!
//! AGENTS.md rule 3: a 429 is never an account fault. Nothing here touches
//! account health; it only paces requests.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

/// Defaults, all overridable by env so a stuck gate can be retuned without a
/// rebuild (the 2026-10-04 incident needed exactly that).
// 12: the measured workspace-wide wall is ~20-24 concurrent (2026-10-04:
// 20x unique-step burst = 20/20; 24x burst hit 3/24 once then recovered).
// Starting under the wall means the AIMD probe discovers the real ceiling
// with single-digit 429s instead of a 48-deep storm on every fresh boot.
const DEFAULT_INITIAL: usize = 12;
/// A real floor, not an inflated one. The 2026-10-04 stall needed MIN=24 as a
/// band-aid only because recovery was slow; with proportional time-based
/// recovery (see `grow_step`) a deep cut climbs back in seconds, so the floor
/// can be an honest "always make some progress" value again.
const DEFAULT_MIN: usize = 8;
const DEFAULT_MAX: usize = 64;
/// How long a gate must stay clean before probing one slot upward. TIME-based,
/// not success-count-based: with a count, a fleet that is itself throttled
/// produces few completions, so recovery stalls exactly when it matters most.
const DEFAULT_RECOVER_MS: u64 = 1_000;
/// How long a lowered cap estimate is trusted before relaxing by one slot.
/// Absolute, NOT a multiple of `recover_ms`: tying it to the recovery tick made
/// it 30ms under a test config, and the two knobs answer different questions
/// ("how fast do we reclaim proven capacity" vs "how long do we believe a 429").
const DEFAULT_RELAX_MS: u64 = 30_000;

/// Grow toward `ceiling` -- an estimate of the workspace's real cap that only
/// ever moves DOWN on evidence.
///
/// Why a ceiling and not just a backoff: Framer's cap is a hard server-side
/// number, not a fair share we are competing for. Once a 429 proves the cap is
/// below L, L is wrong forever (until the upstream itself changes), so the gate
/// should never try L again. Memoryless proportional growth ignored that and
/// relearned the limit from scratch after every cut: measured 2026-10-04 it
/// sawtoothed 8->16->24->32->40->41, took a 429, halved to 8, and repeated
/// every ~8 seconds for the entire run -- a burst of 429s per cycle, paid to
/// rediscover a number already known. The first version of this fix kept a
/// `threshold` at `limit/2` and still oscillated (19 overloads in the
/// convergence test) because that records where recovery RESTARTED, not where
/// the edge IS.
///
/// With a monotone ceiling the 429s are self-extinguishing: each one lowers the
/// estimate, growth is clamped to it, and the gate settles just under the true
/// cap and stays there.
///
/// Fast proportional steps while far below the ceiling (that capacity is
/// proven), +1 probing in the last stretch (unproven -- find the edge for the
/// price of one request, not a burst).
fn grow_step(limit: usize, max: usize, ceiling: usize) -> usize {
    let bound = ceiling.min(max);
    let headroom = bound.saturating_sub(limit);
    if headroom == 0 {
        return 0;
    }
    let probe_zone = (bound / 8).max(1);
    let step = if headroom <= probe_zone { 1 } else { (max / 8).max(1) };
    step.min(headroom)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Tunables, read once per gate construction.
#[derive(Debug, Clone, Copy)]
pub struct GateConfig {
    pub initial: usize,
    pub min: usize,
    pub max: usize,
    pub recover_ms: u64,
    pub relax_ms: u64,
}

impl Default for GateConfig {
    fn default() -> Self {
        let max = env_usize("FRAMERIZE_GATE_MAX", DEFAULT_MAX).max(1);
        let min = env_usize("FRAMERIZE_GATE_MIN", DEFAULT_MIN).clamp(1, max);
        let initial = env_usize("FRAMERIZE_GATE_INITIAL", DEFAULT_INITIAL).clamp(min, max);
        Self {
            initial,
            min,
            max,
            recover_ms: env_u64("FRAMERIZE_GATE_RECOVER_MS", DEFAULT_RECOVER_MS),
            relax_ms: env_u64("FRAMERIZE_GATE_RELAX_MS", DEFAULT_RELAX_MS),
        }
    }
}

struct Gate {
    /// Permits currently in circulation == the effective limit. Resized with
    /// `add_permits` (grow) and acquire-then-forget (shrink), so the semaphore
    /// itself is the single source of truth about capacity.
    sem: Arc<Semaphore>,
    /// Rate smoothing: earliest instant the next acquire may fire. Castle's
    /// docs (2026-10-04): high_activity_account/device/ip signals fire on
    /// "anomalous bursts" relative to baseline — a concurrency cap alone
    /// still allows rate bursts (slots cycling fast), which is the wave
    /// trigger. A per-key min interval smooths it; the queue absorbs wait.
    next_allowed: Instant,
    min_interval: Duration,
    limit: usize,
    /// Permits we decided to remove but could not take immediately because they
    /// were held by in-flight requests. Settled opportunistically as they free.
    /// Without this, a shrink during saturation would silently not happen.
    debt: usize,
    /// When the limit last moved. Recovery is time-based off this.
    changed_at: Instant,
    /// Best estimate of the workspace's real concurrency cap. Starts at
    /// `cfg.max` (no evidence yet) and only ever DECREASES, on 429s. Growth is
    /// clamped to it, which is what makes the 429s self-extinguishing instead
    /// of a permanent sawtooth.
    ceiling: usize,
    /// When `ceiling` was last lowered. A monotone estimate would be wrong
    /// forever if Framer ever RAISES the cap (or if the 429 came from a
    /// transient upstream wobble rather than our own concurrency), so the
    /// estimate relaxes by one slot per quiet window.
    ceiling_at: Instant,
    cfg: GateConfig,
}

impl Gate {
    fn new(cfg: GateConfig, now: Instant) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(cfg.initial)),
            next_allowed: now,
            min_interval: Duration::from_millis(
                std::env::var("FRAMERIZE_GATE_MIN_INTERVAL_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(2500),
            ),
            limit: cfg.initial,
            debt: 0,
            changed_at: now,
            ceiling: cfg.max,
            ceiling_at: now,
            cfg,
        }
    }

    /// Take back up to `debt` permits that have since been released.
    fn settle_debt(&mut self) {
        while self.debt > 0 {
            match self.sem.clone().try_acquire_owned() {
                Ok(p) => {
                    p.forget();
                    self.debt -= 1;
                }
                Err(_) => break,
            }
        }
    }

    fn shrink_to(&mut self, next: usize) {
        if next >= self.limit {
            return;
        }
        self.debt += self.limit - next;
        self.limit = next;
        self.settle_debt();
    }

    fn grow_by(&mut self, n: usize) {
        let next = (self.limit + n).min(self.cfg.max);
        if next == self.limit {
            return;
        }
        let add = next - self.limit;
        // Cancel debt first: handing out a permit we still owe would overshoot.
        let cancelled = add.min(self.debt);
        self.debt -= cancelled;
        let to_add = add - cancelled;
        if to_add > 0 {
            self.sem.add_permits(to_add);
        }
        self.limit = next;
    }
}

/// One gate per workspace key. Concurrency is capped per workspace because that
/// is the scope Framer enforces (429 "too many concurrent AI requests for this
/// workspace").
pub struct ConcurrencyGate {
    gates: Mutex<HashMap<String, Gate>>,
    cfg: GateConfig,
}

impl Default for ConcurrencyGate {
    fn default() -> Self {
        Self::new()
    }
}

impl ConcurrencyGate {
    pub fn new() -> Self {
        Self::with_config(GateConfig::default())
    }

    pub fn with_config(cfg: GateConfig) -> Self {
        Self { gates: Mutex::new(HashMap::new()), cfg }
    }

    /// Wait for a slot in `key`'s workspace. Hold the permit for the whole
    /// request; drop it when the request finishes.
    pub async fn acquire(&self, key: &str) -> OwnedSemaphorePermit {
        let sem = {
            let mut g = self.gates.lock().await;
            let cfg = self.cfg;
            let loaded = self.load_ceiling(key);
            let gate = g.entry(key.to_string()).or_insert_with(|| {
                let mut ng = Gate::new(cfg, Instant::now());
                if let Some(c) = loaded {
                    ng.ceiling = c;
                    ng.limit = ng.limit.min(c);
                }
                ng
            });
            gate.settle_debt();
            // Opportunistic recovery: a saturated gate still calls acquire, so
            // this is enough to climb back without a background task.
            Self::maybe_recover(key, gate, Instant::now());
            let wait = gate.next_allowed.saturating_duration_since(Instant::now());
            gate.next_allowed = Instant::now() + gate.min_interval.max(wait);
            let sem = gate.sem.clone();
            sem
        };
        if wait > Duration::ZERO {
            tokio::time::sleep(wait).await;
        }
        sem.acquire_owned().await.expect("gate semaphore is never closed")
    }

    /// Multiplicative decrease. Call on a 429 ONLY: a 403 risk refusal says
    /// nothing about the concurrency cap, and narrowing on it would conflate
    /// two unrelated signals.
    /// Persisted ceilings file path (survives daemon restarts — the cap is a
    /// server-side fact; re-learning it costs a burst of 429s every boot).
    fn state_path() -> std::path::PathBuf {
        std::env::var("FRAMERIZE_GATE_STATE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from(
                format!("{}/.framerize/gate-state.json", std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))))
    }

    fn persist_ceiling(&self, key: &str, ceiling: usize) {
        let path = Self::state_path();
        let mut map: serde_json::Value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        map[key] = serde_json::json!(ceiling);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, serde_json::to_string(&map).unwrap_or_default());
    }

    fn load_ceiling(&self, key: &str) -> Option<usize> {
        let map: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(Self::state_path()).ok()?,
        )
        .ok()?;
        map[key].as_u64().map(|v| v as usize)
    }

    pub async fn on_overloaded(&self, key: &str) {
        let mut g = self.gates.lock().await;
        let cfg = self.cfg;
        let gate = g.entry(key.to_string()).or_insert_with(|| Gate::new(cfg, Instant::now()));
        // The cap is strictly below the limit that just 429'd, so lower the
        // estimate monotonically. Taken against `limit`, not against the old
        // ceiling, so a cap that we had over-estimated collapses quickly.
        let evidence = gate.limit.saturating_sub(1).max(gate.cfg.min);
        if evidence < gate.ceiling {
            gate.ceiling = evidence;
            gate.ceiling_at = Instant::now();
            self.persist_ceiling(key, evidence);
        }

        // Multiplicative decrease to drain the overshoot fast.
        let next = (gate.limit / 2).max(gate.cfg.min);
        if next < gate.limit {
            gate.shrink_to(next);
            gate.changed_at = Instant::now();
            eprintln!(
                "[gate] {key}: 429 -> limit {} (cap estimate {})",
                gate.limit, gate.ceiling
            );
        }
    }

    /// Additive increase, paced by wall clock.
    pub async fn on_success(&self, key: &str) {
        let mut g = self.gates.lock().await;
        let cfg = self.cfg;
        let gate = g.entry(key.to_string()).or_insert_with(|| Gate::new(cfg, Instant::now()));
        Self::maybe_recover(key, gate, Instant::now());
    }

    fn maybe_recover(key: &str, gate: &mut Gate, now: Instant) {
        // Relax a stale estimate by one slot per quiet window, so a cap that
        // was lowered by a transient wobble -- or that the upstream has since
        // raised -- is not believed forever. One slot per window is slow enough
        // that a real cap costs at most one 429 to rediscover.
        let relax_after = Duration::from_millis(gate.cfg.relax_ms);
        if gate.ceiling < gate.cfg.max && now.duration_since(gate.ceiling_at) >= relax_after {
            gate.ceiling += 1;
            gate.ceiling_at = now;
        }

        if gate.limit >= gate.ceiling.min(gate.cfg.max) {
            return; // at the estimated cap: climbing further only buys 429s
        }
        if now.duration_since(gate.changed_at) < Duration::from_millis(gate.cfg.recover_ms) {
            return;
        }
        let before = gate.limit;
        gate.grow_by(grow_step(gate.limit, gate.cfg.max, gate.ceiling));
        gate.changed_at = now;
        if gate.limit != before {
            eprintln!("[gate] {key}: healthy -> limit {}", gate.limit);
        }
    }

    /// Try to take a slot without waiting. `None` means the gate is full.
    /// Useful for diagnostics and for callers that would rather shed than queue.
    pub async fn acquire_now(&self, key: &str) -> Option<OwnedSemaphorePermit> {
        let sem = {
            let mut g = self.gates.lock().await;
            let cfg = self.cfg;
            let gate = g.entry(key.to_string()).or_insert_with(|| Gate::new(cfg, Instant::now()));
            gate.settle_debt();
            gate.sem.clone()
        };
        sem.try_acquire_owned().ok()
    }

    /// Shrink without recording 429 evidence. Tests only: lets a test set up a
    /// below-max limit to observe pure growth, which `on_overloaded` cannot do
    /// now that it also lowers the cap estimate.
    #[cfg(test)]
    pub async fn shrink_for_test(&self, key: &str, to: usize) {
        let mut g = self.gates.lock().await;
        let cfg = self.cfg;
        let gate = g.entry(key.to_string()).or_insert_with(|| Gate::new(cfg, Instant::now()));
        gate.shrink_to(to);
        gate.changed_at = Instant::now();
    }

    /// Current effective limit, for tests and diagnostics.
    pub async fn limit_of(&self, key: &str) -> usize {
        let mut g = self.gates.lock().await;
        let cfg = self.cfg;
        g.entry(key.to_string()).or_insert_with(|| Gate::new(cfg, Instant::now())).limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fast recovery so tests do not sleep for seconds.
    fn cfg(min: usize, max: usize, recover_ms: u64) -> GateConfig {
        // relax_ms deliberately long so growth tests observe the ceiling as
        // stable; the decay itself is covered by its own test.
        GateConfig { initial: max, min, max, recover_ms, relax_ms: 600_000 }
    }

    fn gate(min: usize, max: usize, recover_ms: u64) -> ConcurrencyGate {
        ConcurrencyGate::with_config(cfg(min, max, recover_ms))
    }

    #[tokio::test]
    async fn starts_at_the_configured_initial_limit() {
        let g = ConcurrencyGate::with_config(GateConfig {
            initial: 10,
            min: 4,
            max: 20,
            recover_ms: 1_000,
            relax_ms: 30_000,
        });
        assert_eq!(g.limit_of("w1").await, 10);
    }

    #[tokio::test]
    async fn halves_on_overload_and_never_goes_below_min() {
        let g = gate(8, 64, 60_000);
        g.on_overloaded("w1").await;
        assert_eq!(g.limit_of("w1").await, 32);
        for _ in 0..20 {
            g.on_overloaded("w1").await;
        }
        assert_eq!(g.limit_of("w1").await, 8, "must floor at min");
    }

    #[tokio::test]
    async fn recovery_is_time_based_not_success_count_based() {
        // The 2026-10-04 bug: recovery needed N successes, so a throttled fleet
        // (few completions) could not climb back out. One success after the
        // window is now enough.
        let g = gate(4, 64, 40);
        g.on_overloaded("w1").await;
        let shrunk = g.limit_of("w1").await;

        for _ in 0..50 {
            g.on_success("w1").await;
        }
        assert_eq!(g.limit_of("w1").await, shrunk, "must not grow inside the window");

        tokio::time::sleep(Duration::from_millis(60)).await;
        g.on_success("w1").await;
        // Growth is PROPORTIONAL to the ceiling (max/8), not a single slot, so
        // recovery time does not depend on how deep the cut was. A +1 step is
        // what made a halved gate take 32 ticks to climb back.
        //
        // `shrunk` IS the threshold after one 429, so this first step probes
        // by 1 -- reclaiming known-good capacity fast only applies below it.
        let ceiling_after_one_429 = 64 - 1; // evidence: cap is below 64
        assert_eq!(
            g.limit_of("w1").await,
            shrunk + grow_step(shrunk, 64, ceiling_after_one_429)
        );
    }

    #[tokio::test]
    async fn the_gate_converges_instead_of_sawtoothing() {
        // THE 2026-10-04 BUG. With memoryless proportional growth the gate
        // relearned the ceiling forever: 8->16->24->32->40 (429) ->8, every
        // ~8s for the whole run, paying a burst of 429s per cycle.
        //
        // Simulate a workspace whose true cap is 20: any limit above it 429s.
        const TRUE_CAP: usize = 20;
        let g = gate(8, 64, 1);
        let mut overloads = 0;

        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(2)).await;
            if g.limit_of("w1").await > TRUE_CAP {
                g.on_overloaded("w1").await;
                overloads += 1;
            } else {
                g.on_success("w1").await;
            }
        }

        // Convergence means the 429s stop happening, not merely that they are
        // survivable. A sawtooth would keep collecting them indefinitely.
        let settled = g.limit_of("w1").await;
        assert!(
            settled <= TRUE_CAP && settled >= 8,
            "should settle just under the true cap, got {settled}"
        );
        assert!(
            overloads < 12,
            "should stop rediscovering the ceiling; took {overloads} overloads"
        );
    }

    #[tokio::test]
    async fn probing_is_cautious_near_the_cap_and_fast_below_it() {
        // Far below the estimated cap, capacity is proven -- reclaim it in big
        // steps. Near it, probe by one so finding the edge costs one request
        // rather than a burst. At or above it, never grow.
        assert_eq!(grow_step(8, 64, 24), 8, "far below the cap: proportional");
        assert_eq!(grow_step(22, 64, 24), 1, "inside the probe zone: +1");
        assert_eq!(grow_step(24, 64, 24), 0, "at the cap: hold");
        assert_eq!(grow_step(40, 64, 24), 0, "above the cap: never grow");
        assert_eq!(grow_step(8, 64, 64), 8, "no 429 yet: climb freely");
        assert_eq!(grow_step(64, 64, 64), 0, "no headroom at max");
        // Growth never jumps past the estimate.
        assert!(20 + grow_step(20, 64, 24) <= 24, "must not overshoot the cap");
    }

    #[tokio::test]
    async fn growth_stops_at_max_when_no_429_was_ever_seen() {
        let g = gate(2, 4, 1);
        g.shrink_for_test("w1", 2).await;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(2)).await;
            g.on_success("w1").await;
        }
        assert_eq!(g.limit_of("w1").await, 4, "no evidence of a lower cap");
    }

    #[tokio::test]
    async fn growth_never_returns_to_a_limit_that_429d() {
        // A 429 at 4 is proof the cap is below 4. Climbing back to 4 would
        // guarantee another 429 -- which is precisely the loop that ran for
        // 7 minutes on 2026-10-04.
        let g = gate(2, 4, 1);
        g.on_overloaded("w1").await;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(2)).await;
            g.on_success("w1").await;
        }
        let settled = g.limit_of("w1").await;
        assert!(settled < 4, "must stay under the limit that 429'd, got {settled}");
        assert_eq!(settled, 3, "should reclaim everything proven safe");
    }

    #[tokio::test]
    async fn a_lowered_cap_estimate_relaxes_so_it_is_not_believed_forever() {
        // A monotone-only ceiling is a trap: if the 429 came from a transient
        // upstream wobble, or Framer later raises the cap, the gate would be
        // stuck at the lower number for the life of the process.
        let cfg = GateConfig { initial: 8, min: 2, max: 8, recover_ms: 1, relax_ms: 10 };
        let g = ConcurrencyGate::with_config(cfg);

        g.on_overloaded("w1").await; // evidence: cap < 8, so ceiling = 7
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(3)).await;
            g.on_success("w1").await;
        }
        // Enough quiet windows passed that the estimate walked back to max.
        assert_eq!(g.limit_of("w1").await, 8, "a stale cap estimate must relax");
    }

    #[tokio::test]
    async fn workspaces_are_independent() {
        let g = gate(4, 64, 60_000);
        g.on_overloaded("w1").await;
        assert_eq!(g.limit_of("w1").await, 32);
        assert_eq!(g.limit_of("w2").await, 64);
    }

    #[tokio::test]
    async fn permits_bound_concurrency_and_release_on_drop() {
        let g = Arc::new(gate(4, 4, 60_000));
        let mut held = Vec::new();
        for _ in 0..4 {
            held.push(g.acquire("w1").await);
        }
        let g2 = g.clone();
        let waiter = tokio::spawn(async move { g2.acquire("w1").await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiter.is_finished(), "gate let a 5th request through a 4-wide limit");

        held.pop();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), waiter).await.is_ok(),
            "waiter never acquired after a permit was released",
        );
    }

    #[tokio::test]
    async fn shrinking_while_saturated_is_honoured_as_permits_free() {
        // The subtle one: shrink cannot take permits that are currently held, so
        // it records debt and settles later. Without that, a 429 during
        // saturation would not actually reduce concurrency.
        let g = Arc::new(gate(1, 4, 60_000));
        let mut held = Vec::new();
        for _ in 0..4 {
            held.push(g.acquire("w1").await);
        }
        g.on_overloaded("w1").await; // 4 -> 2, but all 4 are in flight
        assert_eq!(g.limit_of("w1").await, 2);

        // Release everything; the two owed permits must be absorbed, leaving 2.
        held.clear();
        let _ = g.acquire("w1").await;
        let a = g.acquire("w1").await;
        drop(a);

        let g2 = g.clone();
        let waiter = tokio::spawn(async move { g2.acquire("w1").await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        // One permit is still held by the first acquire above, so with a limit
        // of 2 exactly one more may pass and a third must block.
        let _ = waiter.await;
        assert_eq!(g.limit_of("w1").await, 2);
    }

    #[tokio::test]
    async fn growth_cancels_outstanding_debt_before_adding_permits() {
        let g = Arc::new(gate(1, 8, 1));
        let held: Vec<_> = {
            let mut v = Vec::new();
            for _ in 0..8 {
                v.push(g.acquire("w1").await);
            }
            v
        };
        g.on_overloaded("w1").await; // 8 -> 4, all held, so debt = 4
        assert_eq!(g.limit_of("w1").await, 4);

        tokio::time::sleep(Duration::from_millis(3)).await;
        g.on_success("w1").await; // 4 -> 5 by cancelling debt, NOT adding
        assert_eq!(g.limit_of("w1").await, 5);

        drop(held);
        // Drain the gate, HOLDING each permit: the count of simultaneously
        // held permits must converge to the limit, never exceed it. (Dropping
        // each permit inside the loop would just re-acquire the same slot
        // forever and prove nothing.)
        let mut drained = Vec::new();
        for _ in 0..16 {
            match g.acquire_now("w1").await {
                Some(p) => drained.push(p),
                None => break,
            }
        }
        assert_eq!(drained.len(), 5, "gate capacity must equal its limit");
    }
}
