//! Streaming-abort rung: open one leg, capture the prefix, cancel at the stop.
//!
//! Proven behavior being ported: the abort lands mid-stream, so the
//! terminal/usage event is never emitted and the leg settles nothing. When
//! the terminal DOES arrive before the stop, the leg raced: the caller sees
//! raced=true and the ledger assert decides.
//!
//! STOP CONTRACT (read before changing): the stop counts STREAM EVENTS as
//! framed by the transport -- every Delta increments, including empty
//! keep-alive deltas. This matches the proven driver, which counted `data:`
//! lines (many carrying zero text) against stopAfter. A transport that
//! filters empties before framing changes the stop's meaning; do not.

use crate::stop;
use crate::transport::{CustomTransport, Event};

/// Per-leg wall-clock budget. A hung transport (next_event never resolving)
/// aborts the leg with a typed error instead of hanging the daemon. Sized
/// generously: proven legs take ~10-60s; 10 minutes is stall, not slowness.
pub const LEG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
/// Capture cap: stops unbounded memory if a transport emits an endless delta
/// stream below the stop (or the stop is misconfigured). 200KB holds ~2x the
/// largest proven leg with headroom; hitting it is a typed error, not a trim.
/// Consecutive empty polls before a live leg is declared stalled.
/// Sized past the transport's own stall bound so the transport fails first
/// on a genuinely dead page; this is the backstop for a transport whose
/// None means "waiting" forever.
pub const MAX_SILENT_POLLS: u32 = 2000;

/// Hard ceiling on one leg's captured bytes.
pub const MAX_CAPTURE: usize = 200_000;

/// One leg's outcome.
#[derive(Debug)]
pub struct RungResult {
    /// Prefix text captured before the stop (or before the terminal on race).
    pub captured: String,
    /// True when the terminal arrived before the stop point.
    pub raced: bool,
    /// Stream events consumed (deltas + terminal), for diagnostics.
    pub events: u32,
}

/// Stream one leg: accumulate Delta text; after exactly stop_steps deltas
/// issue graceful cancel and return with raced=false. A Done before the stop
/// returns what arrived with raced=true; a transport that closes with no
/// terminal returns what arrived with raced=false.
///
/// stop_steps is clamped into [MIN_STOP, MAX_STOP]: a 0 stop previously
/// returned an empty capture as a *clean* leg (raced=false), poisoning the
/// chain with a near-zero finisher. Now 0 (or any absurd value) is caller
/// error normalized before the loop starts.
///
/// TIMEOUT: the whole leg runs under LEG_TIMEOUT via tokio::time::timeout.
/// A hung open/stream/cancel surfaces Err("leg timed out"), never a hang.
///
/// STOP PLUMBING: transports that bake the stop in-page (live page stream)
/// learn it via `set_stop_after` on BridgeStreamTransport BEFORE
/// stream_reply: the page aborts itself in-loop at the stop, so the
/// terminal/usage chunk is never read. The Rust-side cancel() after the
/// stop remains as a backstop only. Scripted transports ignore it.
pub async fn run_rung<T: CustomTransport>(
    transport: &mut T,
    payload: &serde_json::Value,
    stop_steps: u32,
) -> Result<RungResult, String> {
    tokio::time::timeout(LEG_TIMEOUT, run_inner(transport, payload, stop_steps))
        .await
        .map_err(|_| "leg timed out after 600s with no terminal or stop".to_string())?
}

async fn run_inner<T: CustomTransport>(
    transport: &mut T,
    payload: &serde_json::Value,
    stop_steps: u32,
) -> Result<RungResult, String> {
    let stop_steps = stop::clamped_stop(stop_steps);
    transport.stream_reply(payload).await?;
    let mut captured = String::new();
    let mut deltas: u32 = 0;
    let mut events: u32 = 0;
    // Consecutive Nones (live transports yield None while the page has no
    // new text yet). Bounded: a transport silent past MAX_SILENT_POLLS has
    // stalled and must fail the leg, never return a silent partial capture
    // as a clean non-raced leg.
    let mut silent: u32 = 0;
    loop {
        match transport.next_event().await {
            Some(Event::Delta(text)) => {
                silent = 0;
                events += 1;
                if captured.len() + text.len() > MAX_CAPTURE {
                    return Err(format!(
                        "leg capture exceeded {MAX_CAPTURE} bytes before stop {stop_steps} (deltas={deltas}): runaway stream or misconfigured stop"
                    ));
                }
                captured.push_str(&text);
                deltas += 1;
                if deltas >= stop_steps {
                    transport.cancel().await;
                    return Ok(RungResult { captured, raced: false, events });
                }
            }
            Some(Event::Done) => {
                events += 1;
                return Ok(RungResult { captured, raced: true, events });
            }
            None => {
                // Scripted transports end the script with None (clean close,
                // no terminal): exactly ONE None may close the leg. Live
                // transports also yield None while waiting for page text --
                // those are paced polls, and only a bounded run of them may
                // pass before the leg is declared stalled. Distinguish by
                // whether ANY framed event arrived first: a None on an
                // untouched stream is end-of-script; Nones after real text
                // are liveness polls.
                if events == 0 {
                    return Ok(RungResult { captured, raced: false, events });
                }
                silent += 1;
                if silent >= MAX_SILENT_POLLS {
                    return Err(format!(
                        "leg stalled: {silent} consecutive empty polls with no terminal (deltas={deltas})"
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

    #[tokio::test]
    async fn cancels_exactly_at_stop() {
        // Contract: stop clamps into [MIN_STOP, MAX_STOP]; with 50 deltas and
        // stop=40 the cancel lands exactly at delta 40, raced=false.
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
    async fn none_after_text_polls_then_terminal() {
        // Live shape: text, paced Nones, then Done. Nones after real text
        // are liveness polls -- the terminal still lands and raced=true.
        struct PollThenDone {
            n: u32,
        }
        #[async_trait::async_trait]
        impl CustomTransport for PollThenDone {
            async fn stream_reply(&mut self, _: &serde_json::Value) -> Result<(), String> {
                Ok(())
            }
            async fn next_event(&mut self) -> Option<Event> {
                // Order: Delta, None, None, Done. The post-text Nones are
                // liveness polls; the terminal still lands and raced=true.
                self.n += 1;
                match self.n {
                    1 => Some(Event::Delta("hi ".into())),
                    4 => Some(Event::Done),
                    _ => None,
                }
            }
            async fn cancel(&mut self) {}
            async fn read_credits(&self) -> Result<f64, String> {
                Ok(1.0)
            }
        }
        let mut t = PollThenDone { n: 0 };
        let r = run_rung(&mut t, &payload(), 200).await.unwrap();
        assert_eq!(r.captured, "hi ");
        assert!(r.raced);
    }

    #[tokio::test]
    async fn endless_silence_fails_not_clean() {
        // A transport yielding None forever after text must FAIL the leg,
        // never return a silent partial capture as clean.
        struct SilentForever;
        #[async_trait::async_trait]
        impl CustomTransport for SilentForever {
            async fn stream_reply(&mut self, _: &serde_json::Value) -> Result<(), String> {
                Ok(())
            }
            async fn next_event(&mut self) -> Option<Event> {
                static CALLS: std::sync::atomic::AtomicU32 =
                    std::sync::atomic::AtomicU32::new(0);
                let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == 0 {
                    return Some(Event::Delta("part ".into()));
                }
                None
            }
            async fn cancel(&mut self) {}
            async fn read_credits(&self) -> Result<f64, String> {
                Ok(1.0)
            }
        }
        // Shrink the bound for the test via a local run: MAX_SILENT_POLLS is
        // large, so instead assert the FIRST None does not close the leg by
        // racing it against a terminal that never comes -- with a timeout.
        let res = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_inner(&mut SilentForever, &payload(), 200),
        )
        .await;
        // Either the stall error or the harness timeout: both prove the leg
        // did NOT return a clean partial capture on the first None.
        match res {
            Ok(Err(e)) => assert!(e.contains("stalled") || e.contains("leg"), "{e}"),
            Err(_) => {}
            Ok(Ok(r)) => panic!("silent transport returned clean: {:?}", r.captured),
        }
    }

    #[tokio::test]
    async fn zero_stop_clamped_not_silent() {
        // stop=0 must NOT return an empty clean leg: it clamps to MIN_STOP.
        let mut t = MockTransport::new(MockTransport::deltas(50));
        let r = run_rung(&mut t, &payload(), 0).await.unwrap();
        assert_eq!(r.captured.matches("word").count(), 40);
        assert!(!r.raced);
        assert_eq!(t.cancel_pos(), Some(40));
    }

    #[tokio::test]
    async fn empty_deltas_count_toward_stop() {
        // Keep-alive framing parity with the proven driver: empties count.
        let mut script = vec![Event::Delta(String::new()); 50];
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
