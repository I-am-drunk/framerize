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
const DEFAULT_INITIAL: usize = 48;
const DEFAULT_MIN: usize = 24;
const DEFAULT_MAX: usize = 64;
/// How long a gate must stay clean before probing one slot upward. TIME-based,
/// not success-count-based: with a count, a fleet that is itself throttled
/// produces few completions, so recovery stalls exactly when it matters most.
const DEFAULT_RECOVER_MS: u64 = 3_000;

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
        }
    }
}

struct Gate {
    /// Permits currently in circulation == the effective limit. Resized with
    /// `add_permits` (grow) and acquire-then-forget (shrink), so the semaphore
    /// itself is the single source of truth about capacity.
    sem: Arc<Semaphore>,
    limit: usize,
    /// Permits we decided to remove but could not take immediately because they
    /// were held by in-flight requests. Settled opportunistically as they free.
    /// Without this, a shrink during saturation would silently not happen.
    debt: usize,
    /// When the limit last moved. Recovery is time-based off this.
    changed_at: Instant,
    cfg: GateConfig,
}

impl Gate {
    fn new(cfg: GateConfig, now: Instant) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(cfg.initial)),
            limit: cfg.initial,
            debt: 0,
            changed_at: now,
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
            let gate = g.entry(key.to_string()).or_insert_with(|| Gate::new(cfg, Instant::now()));
            gate.settle_debt();
            // Opportunistic recovery: a saturated gate still calls acquire, so
            // this is enough to climb back without a background task.
            Self::maybe_recover(key, gate, Instant::now());
            gate.sem.clone()
        };
        sem.acquire_owned().await.expect("gate semaphore is never closed")
    }

    /// Multiplicative decrease. Call on a 429 ONLY: a 403 risk refusal says
    /// nothing about the concurrency cap, and narrowing on it would conflate
    /// two unrelated signals.
    pub async fn on_overloaded(&self, key: &str) {
        let mut g = self.gates.lock().await;
        let cfg = self.cfg;
        let gate = g.entry(key.to_string()).or_insert_with(|| Gate::new(cfg, Instant::now()));
        let next = (gate.limit / 2).max(gate.cfg.min);
        if next < gate.limit {
            gate.shrink_to(next);
            gate.changed_at = Instant::now();
            eprintln!("[gate] {key}: 429 -> limit {}", gate.limit);
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
        if gate.limit >= gate.cfg.max {
            return;
        }
        if now.duration_since(gate.changed_at) < Duration::from_millis(gate.cfg.recover_ms) {
            return;
        }
        gate.grow_by(1);
        gate.changed_at = now;
        eprintln!("[gate] {key}: healthy -> limit {}", gate.limit);
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
        GateConfig { initial: max, min, max, recover_ms }
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
        assert_eq!(g.limit_of("w1").await, shrunk + 1);
    }

    #[tokio::test]
    async fn growth_stops_at_max() {
        let g = gate(2, 4, 1);
        g.on_overloaded("w1").await;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(2)).await;
            g.on_success("w1").await;
        }
        assert_eq!(g.limit_of("w1").await, 4);
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
