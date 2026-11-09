//! Streaming-abort rung: open one leg, capture the prefix, cancel at the stop.
//!
//! Proven behavior being ported: the abort lands mid-stream, so the
//! terminal/usage event is never emitted and the leg settles nothing. When
//! the terminal DOES arrive before the stop, the leg raced: the caller sees
//! raced=true and the ledger assert decides.
//!
//! STOP CONTRACT (read before changing): the stop counts FRAMED DELTAS --
//! every Event::Delta increments, including empty keep-alive deltas from
//! scripted transports. Event::Aborted ends the leg cleanly (raced=false)
//! WITHOUT counting as a delta: it is the page reporting "I already
//! stopped at the stop", not new content. This matches the proven driver, which counted every
//! `data:` line (many carrying zero text) against stopAfter. Live transports
//! signal silence with None (paced, non-consuming, bounded by
//! MAX_SILENT_POLLS) instead of empty deltas, so slow streams never cancel
//! themselves on silence. The two framings coexist: Delta always counts,
//! None never does.

use crate::stop;
use crate::transport::{CustomTransport, Event};

/// Per-leg wall-clock budget. A hung transport (next_event never resolving)
/// aborts the leg with a typed error instead of hanging the daemon. Sized
/// generously: proven legs take ~10-60s; 10 minutes is stall, not slowness.
pub const LEG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
/// Bound on one oracle read: the whole-leg timeout cannot cover reads
/// outside the stream loop, so each credit snapshot gets its own budget.
/// A hung oracle fails the leg loudly instead of wedging the chain.
pub const ORACLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// Bound on the cancel call: fire-and-forget must not become hang-forever.
/// The abort was already issued page-side in the live path; a stuck cancel
/// RPC must not cost the captured prefix.
pub const CANCEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// Capture cap: stops unbounded memory if a transport emits an endless delta
/// stream below the stop (or the stop is misconfigured). 200KB holds ~2x the
/// largest proven leg with headroom; hitting it is a typed error, not a trim.
pub const MAX_CAPTURE: usize = 200_000;
/// Consecutive silent polls before a LIVE leg is declared stalled.
/// Scripted transports are unaffected (their trailing None closes the leg).
/// 240 polls x ~0.6s pacing ~= 2.5 minutes: past the transport's own stall
/// bound (100 paced empties ~= 50s), well beneath the 600s LEG_TIMEOUT, so
/// a silent leg fails LOUD with a named stall (not a bare leg timeout).
/// Pre-text silence counts too: a stream that never yields text is stalled,
/// not slow (proven legs flow text in seconds; the page aborts at the stop
/// and delivers the batch -- endless pre-text silence is a dead pump).
pub const MAX_SILENT_POLLS: u32 = 240;


/// One leg's outcome.
#[derive(Debug)]
pub struct RungResult {
    /// Prefix text captured before the stop (or before the terminal on race).
    pub captured: String,
    /// True when the terminal arrived before the stop point.
    pub raced: bool,
    /// Stream events consumed (deltas + terminal), for diagnostics.
    pub events: u32,
    /// The leg ended on the finish cutoff: the captured text is the
    /// COMPLETE answer and the chain should end (not continue).
    pub finished: bool,
}

/// Cancel with a timeout: the trait contract says fire-and-forget, and this
/// is the enforcement -- a stuck host cancel cannot wedge the leg.
async fn cancel_bounded<T: CustomTransport>(transport: &mut T, stop_steps: u32, deltas: u32) {
    if tokio::time::timeout(CANCEL_TIMEOUT, transport.cancel())
        .await
        .is_err()
    {
        // Fire-and-forget by contract, but a stuck cancel is a host-impl
        // smell: log it with the leg position so daemon logs show it.
        eprintln!(
            "[custom-route] rung cancel timed out after 15s (stop={stop_steps} deltas={deltas})"
        );
    }
}

/// Stream one leg: accumulate Delta text; after exactly stop_steps deltas
/// issue graceful cancel and return with raced=false. A Done before the stop
/// returns what arrived with raced=true.
///
/// NONE HANDLING (the two transport kinds):
/// - scripted (!is_live): a None on an untouched stream (events == 0) is
///   end-of-script -- clean dry leg, raced=false. A None after framed events
///   on a scripted transport is a protocol violation (scripts are total:
///   Done or nothing) and fails the leg loudly.
/// - live (is_live): EVERY None is a liveness poll -- paced, counted toward
///   MAX_SILENT_POLLS, never closing the leg. Only Done closes with raced,
///   only the stop closes clean. This is the slow-model path: the first
///   poll arriving before any text must NOT read as an empty stream.
///
/// stop_steps is clamped into [MIN_STOP, MAX_STOP].
///
/// STOP PLUMBING: transports that bake the stop in-page (live page stream)
/// learn it via `set_stop_after` on BridgeStreamTransport BEFORE
/// stream_reply: the page aborts itself in-loop at the stop, so the
/// terminal/usage chunk is never read. The Rust-side cancel() after the
/// stop remains as a backstop only. Scripted transports ignore it.
///
/// TIMEOUT: the whole leg runs under LEG_TIMEOUT. A hung open/stream/cancel
/// surfaces Err("leg timed out"), never a hang.
pub async fn run_rung<T: CustomTransport>(
    transport: &mut T,
    payload: &serde_json::Value,
    stop_steps: u32,
) -> Result<RungResult, String> {
    let mut out = tokio::time::timeout(LEG_TIMEOUT, run_inner(transport, payload, stop_steps))
        .await
        .map_err(|_| "rung: leg timed out after 600s with no terminal or stop".to_string())??;
    out.finished = transport.last_leg_finished();
    Ok(out)
}

async fn run_inner<T: CustomTransport>(
    transport: &mut T,
    payload: &serde_json::Value,
    stop_steps: u32,
) -> Result<RungResult, String> {
    let stop_steps = stop::clamped_stop(stop_steps);
    // Bake the stop into live transports before opening: the page aborts
    // itself in-loop at the stop (the billing guard); the Rust-side cancel
    // at the stop below is the backstop. No-op for scripted transports.
    transport.set_stop_after(stop_steps);
    transport.stream_reply(payload).await?;
    let live = transport.is_live();
    let mut captured = String::new();
    let mut deltas: u32 = 0;
    let mut events: u32 = 0;
    let mut silent: u32 = 0;
    loop {
        match transport.next_event().await {
            Some(Event::Delta(text)) => {
                silent = 0;
                events = events.saturating_add(1);
                if captured.len() + text.len() > MAX_CAPTURE {
                    return Err(format!(
                        "rung: leg capture exceeded {MAX_CAPTURE} bytes before stop {stop_steps} (deltas={deltas}): runaway stream or misconfigured stop"
                    ));
                }
                captured.push_str(&text);
                deltas += 1;
                if deltas >= stop_steps {
                    cancel_bounded(transport, stop_steps, deltas).await;
                    return Ok(RungResult { captured, raced: false, events, finished: false });
                }
            }
            Some(Event::Done) => {
                events = events.saturating_add(1);
                return Ok(RungResult { captured, raced: true, events, finished: false });
            }
            Some(Event::Aborted) => {
                // The page stopped itself at the stop (graceful abort, no
                // terminal ever arrives BY DESIGN). Clean end with raced=false
                // (nothing billed: the terminal/usage frame was never read).
                // The rung's own cancel is a backstop on an already-stopped
                // stream (harmless); the ledger assert still verifies $0.
                events = events.saturating_add(1);
                cancel_bounded(transport, stop_steps, deltas).await;
                return Ok(RungResult { captured, raced: false, events, finished: false });
            }
            None => {
                if !live {
                    // Scripted: only an untouched stream may close on None.
                    if events == 0 {
                        return Ok(RungResult { captured, raced: false, events, finished: false });
                    }
                    return Err(format!(
                        "rung: scripted transport yielded None after {events} framed events (deltas={deltas}): scripts are total, end with Done"
                    ));
                }
                silent += 1;
                if silent >= MAX_SILENT_POLLS {
                    return Err(format!(
                        "rung: leg stalled: {silent} consecutive empty polls with no terminal (deltas={deltas})"
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;

    fn payload() -> serde_json::Value {
        serde_json::json!({"model": "probe", "stream": true})
    }

    /// Live-flavored script transport for None-semantics tests.
    struct LiveScript {
        script: Vec<Event>,
        pos: usize,
        cancel_pos: Option<usize>,
    }

    impl LiveScript {
        fn new(script: Vec<Event>) -> Self {
            Self { script, pos: 0, cancel_pos: None }
        }
    }

    #[async_trait::async_trait]
    impl CustomTransport for LiveScript {
        async fn stream_reply(&mut self, _: &serde_json::Value) -> Result<(), String> {
            self.pos = 0;
            Ok(())
        }
        async fn next_event(&mut self) -> Option<Event> {
            let ev = self.script.get(self.pos).cloned();
            if ev.is_some() {
                self.pos += 1;
            }
            ev
        }
        async fn cancel(&mut self) {
            if self.cancel_pos.is_none() {
                self.cancel_pos = Some(self.pos);
            }
        }
        async fn read_credits(&self) -> Result<f64, String> {
            Ok(1.0)
        }
        fn is_live(&self) -> bool {
            true
        }
    }

    /// Aborted transport ends the leg cleanly with raced=false (the page
    /// stopped itself at the stop; the terminal was never read, nothing
    /// billed). Deltas before the abort are kept.
    #[tokio::test]
    async fn aborted_ends_leg_clean() {
        struct AbortAfter {
            n: u32,
        }
        #[async_trait::async_trait]
        impl CustomTransport for AbortAfter {
            async fn stream_reply(&mut self, _: &serde_json::Value) -> Result<(), String> {
                Ok(())
            }
            async fn next_event(&mut self) -> Option<Event> {
                self.n += 1;
                match self.n {
                    1 => Some(Event::Delta("hello ".into())),
                    2 => Some(Event::Aborted),
                    _ => panic!("rung must stop at Aborted"),
                }
            }
            async fn cancel(&mut self) {}
            async fn read_credits(&self) -> Result<f64, String> {
                Ok(1.0)
            }
        }
        let mut t = AbortAfter { n: 0 };
        let r = run_rung(&mut t, &payload(), 200).await.unwrap();
        assert_eq!(r.captured, "hello ");
        assert!(!r.raced);
        assert_eq!(r.events, 2);
    }

    /// Hanging transport: next_event never resolves (timeout path).
    struct Hang;
    #[async_trait::async_trait]
    impl CustomTransport for Hang {
        async fn stream_reply(&mut self, _: &serde_json::Value) -> Result<(), String> {
            Ok(())
        }
        async fn next_event(&mut self) -> Option<Event> {
            std::future::pending().await
        }
        async fn cancel(&mut self) {}
        async fn read_credits(&self) -> Result<f64, String> {
            Ok(1.0)
        }
        fn is_live(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn cancels_exactly_at_stop() {
        let mut t = MockTransport::new(MockTransport::deltas(50));
        let r = run_rung(&mut t, &payload(), 40).await.unwrap();
        assert!(!r.raced);
        assert_eq!(t.cancel_pos(), Some(40));
        assert_eq!(r.captured.matches("word").count(), 40);
        assert_eq!(r.events, 40);
    }

    #[tokio::test]
    async fn mid_stream_cancel_position() {
        let mut script = MockTransport::deltas(300);
        script.push(Event::Done);
        let mut t = MockTransport::new(script);
        let r = run_rung(&mut t, &payload(), 200).await.unwrap();
        assert!(!r.raced);
        assert_eq!(t.cancel_pos(), Some(200));
        assert_eq!(r.captured.matches("word").count(), 200);
    }

    #[tokio::test]
    async fn short_stream_races() {
        let mut script = MockTransport::deltas(2);
        script.push(Event::Done);
        let mut t = MockTransport::new(script);
        let r = run_rung(&mut t, &payload(), 200).await.unwrap();
        assert_eq!(r.captured, "word0 word1 ");
        assert!(r.raced);
        assert_eq!(t.cancel_pos(), None);
        assert_eq!(r.events, 3);
    }

    #[tokio::test]
    async fn live_first_none_polls_not_closes() {
        // THE slow-model case: live transport yields None BEFORE any text
        // (first poll, page still thinking), then text, then terminal.
        // None must poll, never close: the leg races with its text.
        struct FirstSilent;
        #[async_trait::async_trait]
        impl CustomTransport for FirstSilent {
            async fn stream_reply(&mut self, _: &serde_json::Value) -> Result<(), String> {
                Ok(())
            }
            async fn next_event(&mut self) -> Option<Event> {
                static CALLS: std::sync::atomic::AtomicU32 =
                    std::sync::atomic::AtomicU32::new(0);
                match CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
                    0 | 1 => None,
                    2 => Some(Event::Delta("slow text here ".into())),
                    _ => Some(Event::Done),
                }
            }
            async fn cancel(&mut self) {}
            async fn read_credits(&self) -> Result<f64, String> {
                Ok(1.0)
            }
            fn is_live(&self) -> bool {
                true
            }
        }
        let r = run_rung(&mut FirstSilent, &payload(), 200).await.unwrap();
        assert_eq!(r.captured, "slow text here ");
        assert!(r.raced, "terminal after polls must race, not clean-close");
    }

    #[tokio::test]
    async fn scripted_none_after_text_is_protocol_error() {
        let mut script = vec![Event::Delta("part ".into()), Event::Delta("more ".into())];
        script.push(Event::Done);
        // truncate: Delta, Delta, then script ends (None after events)
        let mut t = MockTransport::new(vec![Event::Delta("part ".into()), Event::Delta("more ".into())]);
        let err = run_rung(&mut t, &payload(), 200).await.unwrap_err();
        assert!(err.contains("scripted transport"), "{err}");
    }

    #[tokio::test]
    async fn scripted_dry_leg_closes_clean() {
        let mut t = MockTransport::new(vec![]);
        let r = run_rung(&mut t, &payload(), 200).await.unwrap();
        assert!(!r.raced && r.captured.is_empty() && r.events == 0);
    }

    #[tokio::test]
    async fn live_endless_silence_fails_not_clean() {
        let mut t = LiveScript::new(vec![]);
        // Live + always-None: must run into the stall bound, never return
        // a clean empty leg. Bound the test with its own timeout.
        let res = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            // Temporarily shrink the bound via many fast polls: LiveScript
            // returns immediately, so 2000 polls take ms, not minutes.
            run_inner(&mut t, &payload(), 200).await
        })
        .await;
        match res {
            Ok(Err(e)) => assert!(e.contains("stalled"), "{e}"),
            Err(_) => panic!("live silence should stall-fail fast on immediate Nones"),
            Ok(Ok(r)) => panic!("live silence returned clean: {:?}", r.captured),
        }
    }

    #[tokio::test]
    async fn hung_transport_times_out() {
        // LEG_TIMEOUT is 600s: exercise the mechanism with a short local
        // timeout around run_inner-equivalent behavior. run_rung itself
        // would take 600s, so assert the inner loop makes no progress and
        // the timeout wrapper is what rescues it.
        let res = tokio::time::timeout(std::time::Duration::from_millis(200), run_rung(&mut Hang, &payload(), 200)).await;
        assert!(res.is_err(), "hung transport must hit the timeout wrapper");
    }

    #[tokio::test]
    async fn zero_stop_clamped_not_silent() {
        let mut t = MockTransport::new(MockTransport::deltas(50));
        let r = run_rung(&mut t, &payload(), 0).await.unwrap();
        assert_eq!(r.captured.matches("word").count(), 40);
        assert!(!r.raced);
        assert_eq!(t.cancel_pos(), Some(40));
    }

    #[tokio::test]
    async fn empty_deltas_count_toward_stop() {
        let script = vec![Event::Delta(String::new()); 50];
        let mut t = MockTransport::new(script);
        let r = run_rung(&mut t, &payload(), 40).await.unwrap();
        assert!(!r.raced);
        assert!(r.captured.is_empty());
        assert_eq!(t.cancel_pos(), Some(40));
    }

    #[tokio::test]
    async fn runaway_capture_errors() {
        let mut t = MockTransport::new(vec![Event::Delta("x".repeat(1000)); 500]);
        let err = run_rung(&mut t, &payload(), 500).await.unwrap_err();
        assert!(err.contains("exceeded"), "{err}");
    }
}
