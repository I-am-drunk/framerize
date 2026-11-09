//! Custom-route upstream seam: transport abstraction + typed stream events.
//!
//! Honest mapping to the real path (read 2026-11-08 before writing this):
//! - proxy attempt builds one envelope (project, feature category/name,
//!   model, step, session id, step index, turn id, user prompt, request =
//!   payload), seeds the session jar, then calls the browser bridge ai_post
//!   and gets the whole buffered body back as a status/usage-header/text
//!   triple (see port.rs BrowserBridge).
//! - the chat-chain stream helper frames that body as SSE (data lines,
//!   blank-line dispatch) over the Anthropic wire: message_start (id +
//!   input usage), content_block_delta text deltas, message_delta (stop
//!   reason + output usage = the terminal), message_stop.
//! - nativechain instead scans the buffered SSE for the terminal
//!   response.completed object and errors when it never arrives.
//!
//! The real bridge returns whole bodies today (no mid-stream handle), so a
//! live graceful-cancel is an in-page abort owned by the framer-host impl.
//! This trait models that seam now -- open, pull events, cancel -- so the
//! race (terminal before the stop) is representable and the engine stays
//! unit-testable against MockTransport with zero browser.

use async_trait::async_trait;

/// One framed upstream stream item, dialect-neutral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A content increment. Empty-string deltas are legal keep-alive frames
    /// from scripted transports and COUNT toward the rung stop (parity with
    /// the proven driver, which counted every `data:` line). Live transports
    /// instead signal silence with None (paced, non-consuming) -- see the
    /// rung STOP CONTRACT before changing either side.
    Delta(String),
    /// The usage-bearing terminal arrived (Anthropic message_delta, the chat
    /// usage chunk, response.completed, ...). After this the ledger may settle.
    Done,
    /// The page stopped itself at the stop (graceful abort): no terminal
    /// will ever arrive BY DESIGN (it was never read, so nothing bills),
    /// and the stream is over. Distinct from Done (which means "the
    /// terminal arrived, check the ledger") and from None (which means
    /// "nothing yet, poll again"). Live proof 2026-11-08: without this,
    /// the transport could only say None on a finished stream and the rung
    /// starved 240 polls with deltas=3 while text sat delivered.
    Aborted,
}

/// Upstream transport for one custom-route leg. Implemented by framer-host
/// later; the engine only ever sees this trait (+ MockTransport in tests).
#[async_trait]
pub trait CustomTransport: Send + Sync {
    /// Open the upstream stream for one leg (payload = the native request
    /// body; the impl wraps it in the envelope, mints a token in-page, and
    /// POSTs via the account's own browser).
    async fn stream_reply(&mut self, payload: &serde_json::Value) -> Result<(), String>;
    /// Next framed event.
    ///
    /// None means "nothing available RIGHT NOW" (poll again) on live
    /// transports, or end-of-script on scripted ones. End-of-stream arrives
    /// as Event::Done (terminal observed, may have billed), Event::Aborted
    /// (page stopped itself at the stop, terminal never read, nothing
    /// billed), or Err -- except a scripted transport ending its
    /// script with None on an untouched stream (clean close, no terminal),
    /// which the rung accepts as a dry leg. The rung distinguishes the two
    /// via is_live: live Nones (even pre-first-event) are liveness polls and
    /// NEVER close the leg.
    ///
    /// LIVENESS: the engine wraps legs in LEG_TIMEOUT and bounds silent
    /// polls; an impl that never resolves is timed out, never trusted.
    async fn next_event(&mut self) -> Option<Event>;
    /// Graceful cancel: ask upstream to stop sending. The engine calls this
    /// exactly at the stop point, so it must land BEFORE the terminal.
    /// Fire-and-forget by contract: implementations must not block (the rung
    /// caps the call), and the engine never depends on its outcome.
    async fn cancel(&mut self);

    /// True when the last leg ended on the FINISH CUTOFF (the pump aborted
    /// on the model's finish_reason delta): the captured content is the
    /// COMPLETE answer, so the chain ends instead of continuing. Distinct
    /// from the stop_after cutoff (a length cut = partial answer, chain
    /// continues). Default false: scripted transports never finish-cut.
    fn last_leg_finished(&self) -> bool {
        false
    }
    /// Credits oracle: current balance behind the teams ai-credits endpoint.
    /// Must be total and cheap; the engine calls it twice per attempt under
    /// an oracle timeout.
    async fn read_credits(&self) -> Result<f64, String>;
    /// True for live page-backed transports (silence = paced None polls).
    /// False for scripted transports (trailing None = end-of-script).
    /// The rung's None handling hinges on this -- implement honestly.
    fn is_live(&self) -> bool {
        false
    }
    /// Bake the stop into the next stream_reply (content deltas). Live
    /// page-backed transports forward it to open_stream so the page aborts
    /// itself in-loop; scripted transports ignore it (default no-op).
    fn set_stop_after(&mut self, _stop_after: u32) {}
}

/// Scripted transport for tests: replays a fixed event list and records
/// where (if anywhere) the engine cancelled. Test-only.
#[cfg(test)]
pub struct MockTransport {
    script: Vec<Event>,
    pos: usize,
    cancel_pos: Option<usize>,
    credits: f64,
}

#[cfg(test)]
impl MockTransport {
    /// New scripted transport starting at 100.0 credits.
    pub fn new(script: Vec<Event>) -> Self {
        Self { script, pos: 0, cancel_pos: None, credits: 100.0 }
    }

    /// Override the starting balance.
    pub fn with_credits(mut self, credits: f64) -> Self {
        self.credits = credits;
        self
    }

    /// Event index at which cancel arrived (None = never cancelled).
    pub fn cancel_pos(&self) -> Option<usize> {
        self.cancel_pos
    }

    /// Helper: n numbered text deltas.
    pub fn deltas(n: usize) -> Vec<Event> {
        (0..n).map(|i| Event::Delta(format!("word{i} "))).collect()
    }
}

#[cfg(test)]
#[async_trait]
impl CustomTransport for MockTransport {
    async fn stream_reply(&mut self, _payload: &serde_json::Value) -> Result<(), String> {
        self.pos = 0;
        self.cancel_pos = None;
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
        Ok(self.credits)
    }
}
