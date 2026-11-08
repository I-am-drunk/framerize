//! Custom-route upstream seam: transport abstraction + typed stream events.
//!
//! Honest mapping to the real path (read 2026-11-08 before writing this):
//! - proxy attempt (around proxy.rs:121) builds one envelope (project,
//!   feature category/name, model, step, session id, step index, turn id,
//!   user prompt, request = payload), seeds the session jar, then calls the
//!   browser bridge ai_post and gets the whole buffered body back as a
//!   status/usage-header/text triple (see port.rs BrowserBridge).
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
    /// A content increment (one text delta, one chat delta, ...).
    /// Empty-string deltas are legal (keep-alive frames) and counted by the
    /// rung like any delta -- see rung docs on the stop contract.
    Delta(String),
    /// The usage-bearing terminal arrived (Anthropic message_delta, the chat
    /// usage chunk, response.completed, ...). After this the ledger may settle.
    Done,
}

/// Upstream transport for one custom-route leg. Implemented by framer-host
/// later; the engine only ever sees this trait (+ MockTransport in tests).
#[async_trait]
pub trait CustomTransport: Send + Sync {
    /// Open the upstream stream for one leg (payload = the native request
    /// body; the impl wraps it in the envelope, mints a token in-page, and
    /// POSTs via the account's own browser).
    async fn stream_reply(&mut self, payload: &serde_json::Value) -> Result<(), String>;
    /// Next framed event. None = nothing available RIGHT NOW (poll again).
    /// End-of-stream arrives ONLY as Event::Done or Err. A scripted
    /// transport ends its script with None (clean close, no terminal):
    /// run_rung treats a None on an untouched stream as end-of-script.
    /// A live transport yields None while the page has no new text yet;
    /// run_rung bounds consecutive post-text Nones (MAX_SILENT_POLLS) and
    /// fails a stalled leg loudly -- a silent partial capture must never
    /// read as a clean non-raced leg.
    ///
    /// LIVENESS: the engine wraps every call in a timeout (see rung
    /// LEG_TIMEOUT); an impl that never resolves is cancelled there, never
    /// trusted to return on its own.
    async fn next_event(&mut self) -> Option<Event>;
    /// Graceful cancel: ask upstream to stop sending. The engine calls this
    /// exactly at the stop point, so it must land BEFORE the terminal.
    /// Fire-and-forget by contract: implementations must not block, and the
    /// engine never depends on its outcome.
    async fn cancel(&mut self);
    /// Credits oracle: current balance behind the teams ai-credits endpoint
    /// (see FramerClient usage in port.rs). Must be total and cheap; the
    /// engine calls it twice per attempt.
    async fn read_credits(&self) -> Result<f64, String>;
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
