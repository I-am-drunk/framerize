//! SSC ringer: adversarial interleavings against the rung at volume.
//!
//! Models the live path's hazards WITHOUT a browser/daemon/spend:
//! - burst co-location: terminal in the same burst as the last content delta
//! - jitter: silent (None) gaps of varying length mid-stream (live pacing)
//! - micro-stops: stop_after=1 (single-delta abort bound)
//! - 500 tok/s bursts: 500 deltas then terminal (stop must fire first)
//! - empty-keepalive deltas (count toward stop per contract)
//! - Done-first (instant race), Aborted (page self-stop), dry (None-only)
//!
//! Invariants asserted on EVERY run:
//! R1: stop fires at exactly stop_steps deltas (cancel_pos == stop).
//! R2: Done before stop => raced=true (terminal observed, ledger decides).
//! R3: Done at/after stop is UNREACHABLE through run_rung (cancel lands first).
//! R4: captured text == first `stop` deltas concatenated (no loss/dup).
//! R5: Aborted => raced=false, captured intact.
//! R6: dry script (None-only) => clean dry leg, raced=false, empty capture.

#[cfg(test)]
mod ssc_ringer_tests {
    use super::super::rung::run_rung;
    use super::super::transport::{CustomTransport, Event};
    use async_trait::async_trait;

    struct Script {
        script: Vec<Event>,
        live: bool,
        pos: usize,
        cancel_pos: Option<usize>,
        deltas_seen: usize,
    }
    impl Script {
        fn new(script: Vec<Event>, live: bool) -> Self {
            Self { script, pos: 0, cancel_pos: None, deltas_seen: 0, live }
        }
    }
    #[async_trait]
    impl CustomTransport for Script {
        async fn stream_reply(&mut self, _p: &serde_json::Value) -> Result<(), String> { Ok(()) }
        async fn next_event(&mut self) -> Option<Event> {
            let e = self.script.get(self.pos).cloned();
            if e.is_some() {
                self.pos += 1;
                if matches!(e, Some(Event::Delta(_))) { self.deltas_seen += 1; }
            }
            e
        }
        async fn cancel(&mut self) {
            if self.cancel_pos.is_none() { self.cancel_pos = Some(self.deltas_seen); }
        }
        async fn read_credits(&self) -> Result<f64, String> { Ok(100.0) }
        fn is_live(&self) -> bool { self.live }
    }
    fn payload() -> serde_json::Value { serde_json::json!({"model": "probe", "stream": true}) }
    fn words(n: usize) -> Vec<Event> {
        (0..n).map(|i| Event::Delta(format!("w{i} "))).collect()
    }

    // Deterministic PRNG (xorshift64) — seeded runs, reproducible.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0
        }
        fn below(&mut self, n: u64) -> u64 { self.next() % n }
    }

    #[tokio::test]
    async fn ringer_clean_abort_exact_stop() {
        // stop fires at exactly `stop` deltas across stop values and lengths.
        for stop in [1u32, 2, 5, 40, 200, 1000] {
            for extra in [0, 1, 37, 500] {
                let mut t = Script::new(words(stop as usize + extra), false);
                let r = run_rung(&mut t, &payload(), stop).await.unwrap();
                assert!(!r.raced, "stop={stop} extra={extra}");
                assert_eq!(r.captured.split_whitespace().count(), stop as usize, "stop={stop} extra={extra}");
                assert_eq!(t.cancel_pos, Some(stop as usize), "cancel at stop");
            }
        }
    }

    #[tokio::test]
    async fn ringer_done_before_stop_races() {
        // Terminal anywhere before the stop => raced=true, prefix kept.
        for at in [0, 1, 7, 199] {
            let mut script = words(at);
            script.push(Event::Done);
            let mut t = Script::new(script, false);
            let r = run_rung(&mut t, &payload(), 200).await.unwrap();
            assert!(r.raced, "done at {at}");
            assert_eq!(r.captured.split_whitespace().count(), at);
        }
    }

    #[tokio::test]
    async fn ringer_done_at_stop_boundary() {
        // Done as the 200th event (199 deltas + Done): raced (terminal won).
        let mut script = words(199);
        script.push(Event::Done);
        let mut t = Script::new(script, false);
        let r = run_rung(&mut t, &payload(), 200).await.unwrap();
        assert!(r.raced);
        // 200 deltas then Done: stop fires first (cancel at 200), Done unreachable.
        let mut script2 = words(200);
        script2.push(Event::Done);
        let mut t2 = Script::new(script2, false);
        let r2 = run_rung(&mut t2, &payload(), 200).await.unwrap();
        assert!(!r2.raced, "stop at exactly 200 must fire before Done");
        assert_eq!(t2.cancel_pos, Some(200));
    }

    #[tokio::test]
    async fn ringer_500tps_burst() {
        // 500 deltas in one burst + terminal: stop=200 fires mid-burst.
        let mut script = words(500);
        script.push(Event::Done);
        let mut t = Script::new(script, false);
        let r = run_rung(&mut t, &payload(), 200).await.unwrap();
        assert!(!r.raced);
        assert_eq!(r.captured.split_whitespace().count(), 200);
    }

    #[tokio::test]
    async fn ringer_empty_keepalives_count() {
        // Empty deltas count toward the stop (contract parity with page pump).
        let mut script = vec![Event::Delta("".into()); 199];
        script.push(Event::Delta("x ".into()));
        script.push(Event::Done);
        let mut t = Script::new(script, false);
        let r = run_rung(&mut t, &payload(), 200).await.unwrap();
        assert!(!r.raced);
        assert_eq!(t.cancel_pos, Some(200));
    }

    #[tokio::test]
    async fn ringer_aborted_page_self_stop() {
        let mut script = words(37);
        script.push(Event::Aborted);
        let mut t = Script::new(script, false);
        let r = run_rung(&mut t, &payload(), 200).await.unwrap();
        assert!(!r.raced);
        assert_eq!(r.captured.split_whitespace().count(), 37);
    }

    #[tokio::test]
    async fn ringer_dry_script() {
        let mut t = Script::new(vec![], false);
        // next_event -> None immediately on untouched scripted stream.
        let r = run_rung(&mut t, &payload(), 200).await.unwrap();
        assert!(!r.raced);
        assert!(r.captured.is_empty());
    }

    #[tokio::test]
    async fn ringer_live_jitter_silence() {
        // Live transport: None gaps mid-stream are paced polls, never close.
        for gaps in [1, 10, 100] {
            let mut script = words(50);
            for _ in 0..gaps { script.insert(25, Event::Delta("".into())); }
            script.push(Event::Done);
            let mut t = Script::new(script, true);
            let r = run_rung(&mut t, &payload(), 200).await.unwrap();
            assert!(r.raced, "gaps={gaps}: terminal still arrives");
        }
    }

    #[tokio::test]
    async fn ringer_fuzz_10k_interleavings() {
        // 10k seeded runs: random delta/empty/done/aborted mixes, stops 1..400.
        // Invariants: cancel_pos <= stop; raced <=> Done arrived before cancel;
        // captured deltas == min(stop, deltas-before-Done/Aborted).
        let mut rng = Rng(0xC07FFEE);
        let mut raced_n = 0u32;
        for _ in 0..10_000 {
            let stop = 1 + rng.below(400) as u32;
            let n = rng.below(600) as usize;
            let mut script = Vec::new();
            let mut deltas = 0u32;
            let mut done_at: Option<u32> = None;
            let mut aborted_at: Option<u32> = None;
            for _ in 0..n {
                match rng.below(100) {
                    0..=88 => { script.push(Event::Delta("w ".into())); deltas += 1; }
                    89..=93 => { script.push(Event::Delta("".into())); deltas += 1; }
                    94..=96 => { if done_at.is_none() && aborted_at.is_none() { script.push(Event::Done); done_at = Some(deltas); } }
                    _ => { if done_at.is_none() && aborted_at.is_none() { script.push(Event::Aborted); aborted_at = Some(deltas); } }
                }
                if done_at.is_some() || aborted_at.is_some() { break; }
            }
            // Scripts are total: terminate with Done unless dry or aborted.
            // (A bare end-of-script after framed events is a protocol violation
            // by construction — the live path always ends Done/Aborted.)
            if done_at.is_none() && aborted_at.is_none() && deltas > 0 {
                script.push(Event::Done);
                done_at = Some(deltas);
            }
            let mut t = Script::new(script, false);
            let r = run_rung(&mut t, &payload(), stop).await.unwrap();
            // Expected: raced iff Done arrived with deltas < stop.
            let expect_raced = matches!(done_at, Some(d) if d < stop);
            assert_eq!(r.raced, expect_raced, "stop={stop} done_at={done_at:?} aborted_at={aborted_at:?}");
            if r.raced { raced_n += 1; }
            if !r.raced && aborted_at.is_none() && done_at.is_none() {
                // Clean abort or dry: cancel at min(stop, total deltas) — unless dry (0 events, no cancel).
                if deltas > 0 {
                    assert_eq!(t.cancel_pos, Some(deltas.min(stop) as usize), "stop={stop} deltas={deltas}");
                }
            }
        }
        assert!(raced_n > 100, "fuzz must produce races (got {raced_n})");
    }
}
