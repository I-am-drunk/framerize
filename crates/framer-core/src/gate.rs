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
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

/// Start below the measured 429 threshold, with room to grow.
pub const INITIAL_LIMIT: usize = 48;
/// Never drop below this: some progress always beats a stalled fleet.
pub const MIN_LIMIT: usize = 24;
/// Never exceed: 64 was clean, 100 was not.
pub const MAX_LIMIT: usize = 64;
/// Consecutive successes before probing upward.
const GROW_AFTER: u32 = 4;

struct Gate {
    sem: Arc<Semaphore>,
    limit: usize,
    /// Permits removed to shrink the effective limit, parked here so they can be
    /// handed back when we grow again.
    parked: Vec<OwnedSemaphorePermit>,
    ok_streak: u32,
}

impl Gate {
    fn new() -> Self {
        Self {
            sem: Arc::new(Semaphore::new(MAX_LIMIT)),
            limit: INITIAL_LIMIT,
            parked: Vec::new(),
            ok_streak: 0,
        }
    }
}

/// One gate per workspace key. Concurrency is capped per workspace because that
/// is the scope Framer enforces.
pub struct ConcurrencyGate {
    gates: Mutex<HashMap<String, Gate>>,
}

impl Default for ConcurrencyGate {
    fn default() -> Self {
        Self::new()
    }
}

impl ConcurrencyGate {
    pub fn new() -> Self {
        Self { gates: Mutex::new(HashMap::new()) }
    }

    /// Wait for a slot in `key`'s workspace. Hold the permit for the whole
    /// request; drop it when done.
    pub async fn acquire(&self, key: &str) -> OwnedSemaphorePermit {
        let sem = {
            let mut g = self.gates.lock().await;
            let gate = g.entry(key.to_string()).or_insert_with(Gate::new);
            // Park down to the current limit before handing anything out.
            while gate.parked.len() < MAX_LIMIT - gate.limit {
                match gate.sem.clone().try_acquire_owned() {
                    Ok(p) => gate.parked.push(p),
                    Err(_) => break,
                }
            }
            gate.sem.clone()
        };
        sem.acquire_owned().await.expect("gate semaphore never closes")
    }

    /// Call on a 429: halve the limit (multiplicative decrease).
    pub async fn on_overloaded(&self, key: &str) {
        let mut g = self.gates.lock().await;
        let gate = g.entry(key.to_string()).or_insert_with(Gate::new);
        gate.ok_streak = 0;
        let next = (gate.limit / 2).max(MIN_LIMIT);
        if next < gate.limit {
            gate.limit = next;
            eprintln!("[gate] {key}: 429 -> limit {}", gate.limit);
        }
    }

    /// Call on success: after a streak, probe one slot upward (additive increase).
    pub async fn on_success(&self, key: &str) {
        let mut g = self.gates.lock().await;
        let gate = g.entry(key.to_string()).or_insert_with(Gate::new);
        gate.ok_streak += 1;
        if gate.ok_streak >= GROW_AFTER && gate.limit < MAX_LIMIT {
            gate.ok_streak = 0;
            gate.limit += 1;
            gate.parked.pop(); // return one slot to circulation
            eprintln!("[gate] {key}: healthy -> limit {}", gate.limit);
        }
    }

    #[cfg(test)]
    async fn limit_of(&self, key: &str) -> usize {
        let mut g = self.gates.lock().await;
        g.entry(key.to_string()).or_insert_with(Gate::new).limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn starts_at_initial_limit() {
        let g = ConcurrencyGate::new();
        assert_eq!(g.limit_of("w1").await, INITIAL_LIMIT);
    }

    #[tokio::test]
    async fn halves_on_overload_and_floors_at_min() {
        let g = ConcurrencyGate::new();
        g.on_overloaded("w1").await;
        assert_eq!(g.limit_of("w1").await, INITIAL_LIMIT / 2);
        for _ in 0..20 {
            g.on_overloaded("w1").await;
        }
        assert_eq!(g.limit_of("w1").await, MIN_LIMIT);
    }

    #[tokio::test]
    async fn grows_only_after_a_success_streak_and_caps_at_max() {
        let g = ConcurrencyGate::new();
        g.on_overloaded("w1").await; // 24
        let shrunk = g.limit_of("w1").await;
        for _ in 0..(GROW_AFTER - 1) {
            g.on_success("w1").await;
        }
        assert_eq!(g.limit_of("w1").await, shrunk, "must not grow early");
        g.on_success("w1").await;
        assert_eq!(g.limit_of("w1").await, shrunk + 1);

        for _ in 0..(GROW_AFTER * (MAX_LIMIT as u32) + GROW_AFTER) {
            g.on_success("w1").await;
        }
        assert_eq!(g.limit_of("w1").await, MAX_LIMIT, "must not exceed max");
    }

    #[tokio::test]
    async fn workspaces_are_independent() {
        let g = ConcurrencyGate::new();
        g.on_overloaded("w1").await;
        assert_eq!(g.limit_of("w1").await, INITIAL_LIMIT / 2);
        assert_eq!(g.limit_of("w2").await, INITIAL_LIMIT);
    }

    #[tokio::test]
    async fn permits_bound_concurrency_and_release_on_drop() {
        let g = Arc::new(ConcurrencyGate::new());
        for _ in 0..8 {
            g.on_overloaded("w1").await; // floor it to MIN_LIMIT
        }
        assert_eq!(g.limit_of("w1").await, MIN_LIMIT);

        let mut held = Vec::new();
        for _ in 0..MIN_LIMIT {
            held.push(g.acquire("w1").await);
        }
        // One more must block while all MIN_LIMIT permits are held.
        let g2 = g.clone();
        let waiter = tokio::spawn(async move { g2.acquire("w1").await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!waiter.is_finished(), "gate let an extra request through");

        held.pop(); // release one
        let got = tokio::time::timeout(std::time::Duration::from_secs(2), waiter).await;
        assert!(got.is_ok(), "waiter never acquired after a release");
    }
}
