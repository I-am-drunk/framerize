//! Live custom-route transport (streaming read + graceful cancel + oracle).
//!
//! The honest streaming path (`fetch` + `reader.read()` chunks +
//! `reader.cancel()` at the stop) is the proven page mechanism: the in-page
//! stream runner opens the chat POST with `stream:true`, frames `data:`
//! lines, pulls delta text, and at `events >= stopAfter` runs `ctl.abort()`
//! + `reader.cancel()` -- before the terminal/usage chunk.
//!
//! `PageStream` is the host seam framer-host implements: `open_stream`
//! starts the page fetch WITH the stop baked in and returns a stream id;
//! `drain` pulls newly arrived framed batches; `cancel` performs the
//! page-side graceful `reader.cancel()`; `credits` reads the ledger.
//!
//! `BridgeStreamTransport` drives that seam and stays honest: when no
//! `PageStream` is wired it fails closed (`live page stream not wired`),
//! never hangs (every drain runs under DRAIN_TIMEOUT, paced by
//! DRAIN_POLL_INTERVAL), never serves scripted text as real data.
//!
//! TRUST CONTRACT on StreamBatch (documented for the host impl): `raced`
//! must be true if and only if the batch's events end with the terminal.
//! The transport enforces this -- a batch violating it is rejected as a
//! protocol error rather than trusted (see StreamBatch::check).
//!
//! MEMORY: `pending` holds at most one undrained batch (bounded by
//! MAX_BATCH_EVENTS below); the rung caps total capture independently.

use super::format::{format_tail, Dialect};
use super::transport::{CustomTransport, Event};
use async_trait::async_trait;

/// Bound on one page drain: a hung host call fails the leg, never the daemon.
pub const DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// Pace between empty page polls: the rung awaits real framed text, and a
/// tight loop would burn CPU plus CDP round-trips while the model thinks.
pub const DRAIN_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
/// Cap on events accepted from one page batch: a hostile or garbage host
/// impl must not stuff a million deltas into `pending` in one drain.
pub const MAX_BATCH_EVENTS: usize = 10_000;

/// One drained page-side batch: framed text events plus stream state.
/// `raced` = the terminal arrived before any cancel (the abort window was
/// missed); `aborted` = the page cancelled mid-stream at the stop.
/// INVARIANT: raced => events ends with Done. The transport checks this.
#[derive(Debug)]
pub struct StreamBatch {
    /// Framed events in arrival order (capped at MAX_BATCH_EVENTS).
    pub events: Vec<Event>,
    /// Terminal arrived before cancel (must match a trailing Done).
    pub raced: bool,
    /// Page cancelled mid-stream at the stop.
    pub aborted: bool,
    /// The page aborted on the FINISH cutoff (finish_reason delta): the
    /// captured content is the complete answer. False on a length cut.
    pub finished: bool,
    /// Tool calls accumulated by the page pump (chat tool_calls deltas,
    /// assembled per call index). Only the FINISHED leg's set is complete;
    /// a length-cut leg's partial calls are dropped (a tool call cannot
    /// split across legs).
    pub tool_calls: Vec<serde_json::Value>,
}

impl StreamBatch {
    /// Protocol check: raced batches must carry the terminal; a batch
    /// claiming both raced and aborted is contradictory. Empty raced
    /// batches are impossible (no terminal); empty aborted batches are
    /// legal (cancel landed before any text arrived).
    pub fn check(&self) -> Result<(), String> {
        if self.events.len() > MAX_BATCH_EVENTS {
            return Err(format!(
                "page batch carries {} events, over the {MAX_BATCH_EVENTS} cap",
                self.events.len()
            ));
        }
        if self.raced && self.aborted {
            return Err("page batch claims raced and aborted at once".to_string());
        }
        if self.finished && !self.aborted {
            return Err("page batch claims finished without the abort".to_string());
        }
        if self.raced && !matches!(self.events.last(), Some(Event::Done)) {
            return Err("page batch claims raced with no terminal Done".to_string());
        }
        Ok(())
    }
}

/// The host capability the live runner needs.
#[async_trait]
pub trait PageStream: Send + Sync {
    /// Start the page fetch WITH the stop baked in (stop_after content
    /// deltas). The page aborts itself in-loop at the stop -- see
    /// OPEN_SNIPPET in framer-host -- so the terminal/usage chunk is never
    /// read. Returns a stream id for drain/cancel (cancel remains a
    /// backstop only).
    async fn open_stream(
        &mut self,
        payload: &serde_json::Value,
        dialect: Dialect,
        stop_after: u32,
    ) -> Result<String, String>;
    /// Pull newly arrived framed batches (ALWAYS non-blocking: empty batch
    /// when nothing arrived yet; raced batches carry the terminal).
    async fn drain(&mut self, stream_id: &str) -> Result<StreamBatch, String>;
    /// Page-side graceful `reader.cancel()`. Returns true when the page
    /// confirms the reader was open and is now cancelled.
    async fn cancel(&mut self, stream_id: &str) -> Result<bool, String>;
    /// Ledger read via the in-page credits GET.
    async fn credits(&self) -> Result<f64, String>;
}

/// Live transport over a `PageStream`. `dialect` pins which tail rule from
/// `format_tail` governs the run (kept for the host impl + honest logs).
#[derive(Debug)]
pub struct BridgeStreamTransport<P: PageStream> {
    page: P,
    dialect: Dialect,
    /// Stop baked into the NEXT open_stream call (content deltas). Set via
    /// set_stop_after; defaults to u32::MAX (page runs to terminal -- only
    /// for drains that never set it, never the rung path).
    stop_after: u32,
    stream_id: Option<String>,
    pending: std::collections::VecDeque<Event>,
    terminal_seen: bool,
    /// The last leg ended on the finish cutoff (complete answer captured).
    finished_cut: bool,
    /// The finished leg's complete tool calls (empty for text turns).
    tool_calls: Vec<serde_json::Value>,
    failed: Option<String>,
    /// Consecutive empty drains: stall detection needs MANY in a row (a live
    /// model pauses between tokens); one empty batch is normal, not failure.
    empty_streak: u32,
}

/// Empty drains before the leg is declared stalled. Sized for slow models
/// but well beneath the 600s leg bound: 100 x ~0.5s polls ~= 50s of true
/// silence. (600 here used to BURY every leg failure: 2000 rung polls x
/// ~0.6s ~= 20min of silence budget meant the 600s LEG_TIMEOUT always fired
/// first with a bare timeout and no drain/stall line ever named the cause.)
pub const MAX_EMPTY_DRAINS: u32 = 100;

impl<P: PageStream> BridgeStreamTransport<P> {
    /// New live transport. The debug_assert pins the tail table: every
    /// dialect the engine can run must name its usage event.
    pub fn new(page: P, dialect: Dialect) -> Self {
        let tail = format_tail(dialect);
        debug_assert!(
            !tail.usage_event.is_empty(),
            "dialect tail must name its usage event"
        );
        Self {
            page,
            dialect,
            stop_after: u32::MAX,
            stream_id: None,
            pending: std::collections::VecDeque::new(),
            terminal_seen: false,
            finished_cut: false,
            tool_calls: Vec::new(),
            failed: None,
            empty_streak: 0,
        }
    }

    /// Current dialect (for host-impl logging).
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Bake the stop into the next open_stream: the page aborts itself
    /// in-loop at stop_after content deltas. The rung path sets this before
    /// every leg (see run_rung); without it the page would run to terminal
    /// and the leg would bill.
    pub fn set_stop_after(&mut self, stop_after: u32) {
        self.stop_after = stop_after.max(1);
    }

    /// Test hook: the stop currently baked in (u32::MAX = unset).
    #[cfg(test)]
    pub fn stop_after(&self) -> u32 {
        self.stop_after
    }
}

impl<P: PageStream> BridgeStreamTransport<P> {
    /// Best-effort cancel of the CURRENT page stream, fire-and-forget. Called on
    /// every failure arm inside next_event: a failed leg must not leave its
    /// server-side stream generating toward the terminal unwatched (settle with
    /// nobody attributing it). Logs, never fails.
    async fn cancel_current(&mut self) {
        if let Some(id) = self.stream_id.clone() {
            let _ = self.page.cancel(&id).await;
        }
    }
}

#[async_trait]
impl<P: PageStream> CustomTransport for BridgeStreamTransport<P> {
    fn set_stop_after(&mut self, stop_after: u32) {
        self.set_stop_after(stop_after);
    }

    async fn stream_reply(&mut self, payload: &serde_json::Value) -> Result<(), String> {
        self.pending.clear();
        self.terminal_seen = false;
        self.finished_cut = false;
        self.tool_calls = Vec::new();
        self.failed = None;
        self.empty_streak = 0;
        // Fail-closed default: an UNSET stop (u32::MAX) means "run to terminal" = bill.
        // Every legit caller sets it via set_stop_after (rung path); an unset stop here
        // is a caller bug, and billing is the wrong way to learn about it.
        if self.stop_after == u32::MAX {
            eprintln!("[custom-route] stream_reply with unset stop_after -- clamping to DEFAULT_STOP (caller bug, fail-closed)");
            self.stop_after = crate::stop::DEFAULT_STOP;
        }
        let id = self
            .page
            .open_stream(payload, self.dialect, self.stop_after)
            .await?;
        self.stream_id = Some(id);
        Ok(())
    }

    async fn next_event(&mut self) -> Option<Event> {
        if let Some(ev) = self.pending.pop_front() {
            if ev == Event::Done {
                self.terminal_seen = true;
            }
            return Some(ev);
        }
        if self.terminal_seen || self.failed.is_some() {
            return None;
        }
        let id = self.stream_id.as_deref()?;
        let batch = match tokio::time::timeout(DRAIN_TIMEOUT, self.page.drain(id)).await {
            Ok(Ok(b)) => b,
            Ok(Err(e)) => {
                // DRAIN-VISIBILITY FIX (2026-11-08): a drain ERROR is a leg
                // failure, not a silent poll. Failing here (instead of
                // yielding None) makes a page-side reset surface as a loud
                // named chain error within one poll instead of 2000 silent
                // Nones and a 16-minute stall mystery.
                eprintln!("[custom-route] live page drain error: {e}");
                self.failed = Some(format!("live: page drain failed: {e}"));
                self.cancel_current().await;
                return None;
            }
            Err(_) => {
                eprintln!("[custom-route] live page drain timed out after 120s");
                self.failed = Some("live: page drain timed out after 120s".into());
                self.cancel_current().await;
                return None;
            }
        };
        if let Err(e) = batch.check() {
            // Protocol violation by the host impl: fail the leg LOUDLY with
            // the reason preserved, never silently trust the batch.
            self.failed = Some(format!("live: page protocol violation: {e}"));
            self.cancel_current().await;
            return None;
        }
        // Post-terminal batches are a host-impl bug (terminal_seen is set
        // when Done is YIELDED, so a racing batch arriving between the last
        // yield and this drain still lands here): drop text after Done
        // loudly instead of feeding a second leg's worth of text.
        if self.terminal_seen && !batch.events.is_empty() {
            self.failed = Some("live: page sent events after the terminal Done".into());
            self.cancel_current().await;
            return None;
        }
        if batch.raced {
            self.terminal_seen = true;
        }
        // The page stopped itself at the stop (graceful abort, no terminal
        // will ever arrive BY DESIGN): queue the batch text, then yield
        // Event::Aborted BEHIND it so the rung ends the leg cleanly
        // (raced=false, nothing billed). Live proof 2026-11-08: the pump
        // stopped at 200 lines with text flowing (813 + 43 + 11 chars
        // across the last drains); without an explicit terminal the rung
        // starved 240 polls with deltas=3 on the finished stream.
        if batch.aborted {
            // A finish-cut batch carries the COMPLETE answer: flag it for
            // the chain (which ends instead of continuing) while the event
            // flow stays Aborted (no terminal arrived BY DESIGN).
            self.finished_cut = batch.finished;
            if !batch.tool_calls.is_empty() {
                self.tool_calls = batch.tool_calls.clone();
            }
            self.pending.extend(batch.events);
            self.pending.push_back(Event::Aborted);
            return self.pending.pop_front();
        }
        if batch.events.is_empty() {
            // Empty batch, no terminal: the page had nothing new THIS poll.
            // Normal during slow generation -- only a long streak is a stall.
            self.empty_streak += 1;
            if self.empty_streak >= MAX_EMPTY_DRAINS {
                eprintln!(
                    "[custom-route] live page stream stalled: {} empty drains with no terminal",
                    self.empty_streak
                );
                self.failed = Some(format!(
                    "live: page stream stalled: {} empty drains with no terminal",
                    self.empty_streak
                ));
                self.cancel_current().await;
                return None;
            }
            // Pace the poll loop (no busy spin) and yield NO event: the
            // rung's stop counts only real framed text. An empty keep-alive
            // must never consume stop budget -- a slow stream would
            // otherwise cancel itself on silence, not on length.
            tokio::time::sleep(DRAIN_POLL_INTERVAL).await;
            return None;
        }
        self.empty_streak = 0;
        self.pending.extend(batch.events);
        self.pending.pop_front()
    }

    fn last_leg_finished(&self) -> bool {
        self.finished_cut
    }

    fn take_tool_calls(&mut self) -> Vec<serde_json::Value> {
        std::mem::take(&mut self.tool_calls)
    }

    async fn cancel(&mut self) {
        if let Some(id) = self.stream_id.as_deref() {
            // Outcome intentionally ignored (fire-and-forget contract): a
            // failed page-cancel after our stop is a host-impl matter, and
            // the ledger assert is the real safety net. Logged at the host.
            let _ = self.page.cancel(id).await;
        }
    }

    async fn read_credits(&self) -> Result<f64, String> {
        self.page.credits().await
    }

    fn is_live(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Scripted PageStream: open/drain/cancel/credits from queues.
    struct ScriptPage {
        batches: VecDeque<StreamBatch>,
        credits: f64,
        cancels: usize,
        fail_open: bool,
    }

    impl ScriptPage {
        fn new(batches: Vec<StreamBatch>, credits: f64) -> Self {
            Self {
                batches: batches.into(),
                credits,
                cancels: 0,
                fail_open: false,
            }
        }
    }

    #[async_trait]
    impl PageStream for ScriptPage {
        async fn open_stream(
            &mut self,
            _payload: &serde_json::Value,
            _dialect: Dialect,
            stop_after: u32,
        ) -> Result<String, String> {
            assert!(stop_after >= 1, "stop baked in-page must be >= 1");
            if self.fail_open {
                return Err("live page stream not wired".into());
            }
            Ok("s1".into())
        }

        async fn drain(&mut self, _stream_id: &str) -> Result<StreamBatch, String> {
            Ok(self.batches.pop_front().unwrap_or(StreamBatch {
                events: vec![],
                raced: false,
                aborted: false,
                finished: false,
                tool_calls: Vec::new(),
            }))
        }

        async fn cancel(&mut self, _stream_id: &str) -> Result<bool, String> {
            self.cancels += 1;
            Ok(true)
        }

        async fn credits(&self) -> Result<f64, String> {
            Ok(self.credits)
        }
    }

    fn batch(texts: &[&str], raced: bool) -> StreamBatch {
        // Legacy helper: raced batches are OBSERVED terminals (Done flows
        // in the batch, aborted=false); non-raced batches are plain text
        // (aborted=false here -- the stopped-text path uses batch_full).
        batch_full(texts, raced, false)
    }

    /// Full constructor: the legacy batch() couples aborted=!raced, but the
    /// live page reports them independently (stopped-text batches are
    /// aborted=true with events; raced batches carry Done). Tests for the
    /// stopped-text path need aborted WITHOUT raced.
    fn batch_full(texts: &[&str], raced: bool, aborted: bool) -> StreamBatch {
        let mut events: Vec<Event> =
            texts.iter().map(|t| Event::Delta(t.to_string())).collect();
        if raced {
            events.push(Event::Done);
        }
        StreamBatch {
            events,
            raced,
            aborted,
            finished: false,
                tool_calls: Vec::new(),
        }
    }

    #[tokio::test]
    async fn streams_batches_then_ends() {
        // batch() appends Done when raced=true, so the terminal batch is
        // honest: two deltas, then Done, then end-of-stream.
        let page = ScriptPage::new(
            vec![batch(&["hello ", "world "], false), batch(&[], true)],
            100.0,
        );
        let mut t = BridgeStreamTransport::new(page, Dialect::Chat);
        assert!(t.is_live());
        t.stream_reply(&serde_json::json!({})).await.unwrap();
        assert_eq!(
            t.next_event().await,
            Some(Event::Delta("hello ".into()))
        );
        assert_eq!(
            t.next_event().await,
            Some(Event::Delta("world ".into()))
        );
        assert_eq!(t.next_event().await, Some(Event::Done));
        assert_eq!(t.next_event().await, None);
        assert_eq!(t.read_credits().await.unwrap(), 100.0);
    }

    #[tokio::test]
    async fn raced_without_terminal_rejected() {
        // A host batch claiming raced but carrying NO Done violates the
        // trust contract: the leg fails loudly, no silent terminal.
        let page = ScriptPage::new(
            vec![StreamBatch {
                events: vec![Event::Delta("x ".into())],
                raced: true,
                aborted: false,
                finished: false,
                tool_calls: Vec::new(),
            }],
            100.0,
        );
        let mut t = BridgeStreamTransport::new(page, Dialect::Chat);
        t.stream_reply(&serde_json::json!({})).await.unwrap();
        assert_eq!(t.next_event().await, None);
        assert!(t.failed.as_deref().unwrap_or("").contains("protocol violation"));
    }

    #[tokio::test]
    async fn honest_raced_batch_flows() {
        let page = ScriptPage::new(
            vec![batch(&["hello "], false), batch(&["world "], true)],
            100.0,
        );
        let mut t = BridgeStreamTransport::new(page, Dialect::Chat);
        t.stream_reply(&serde_json::json!({})).await.unwrap();
        assert_eq!(t.next_event().await, Some(Event::Delta("hello ".into())));
        assert_eq!(t.next_event().await, Some(Event::Delta("world ".into())));
        assert_eq!(t.next_event().await, Some(Event::Done));
        assert_eq!(t.next_event().await, None);
    }

    #[tokio::test]
    async fn oversize_batch_rejected() {
        let big = StreamBatch {
            events: vec![Event::Delta("x".into()); MAX_BATCH_EVENTS + 1],
            raced: false,
            aborted: false,
            finished: false,
                tool_calls: Vec::new(),
        };
        let page = ScriptPage::new(vec![big], 100.0);
        let mut t = BridgeStreamTransport::new(page, Dialect::Chat);
        t.stream_reply(&serde_json::json!({})).await.unwrap();
        assert_eq!(t.next_event().await, None);
        assert!(t.failed.as_deref().unwrap_or("").contains("over the"), "{:?}", t.failed);
    }

    #[tokio::test]
    async fn empty_batches_yield_nothing_then_stall() {
        // Empty batches yield NOTHING (paced polls), not fake closure and
        // not stop-consuming keep-alives.
        struct FewEmpty {
            n: u32,
        }
        #[async_trait]
        impl PageStream for FewEmpty {
            async fn open_stream(&mut self, _: &serde_json::Value, _: Dialect, _: u32) -> Result<String, String> {
                Ok("s".into())
            }
            async fn drain(&mut self, _: &str) -> Result<StreamBatch, String> {
                if self.n == 0 {
                    return Ok(batch(&["late text "], false));
                }
                self.n -= 1;
                Ok(StreamBatch { events: vec![], raced: false, aborted: false, finished: false, tool_calls: Vec::new() })
            }
            async fn cancel(&mut self, _: &str) -> Result<bool, String> {
                Ok(true)
            }
            async fn credits(&self) -> Result<f64, String> {
                Ok(1.0)
            }
        }
        let mut t = BridgeStreamTransport::new(FewEmpty { n: 3 }, Dialect::Chat);
        t.stream_reply(&serde_json::json!({})).await.unwrap();
        // Empty page polls yield NO event (paced 500ms sleeps), so the
        // rung's stop counts only real framed text; the late text still
        // arrives on the fourth poll.
        assert_eq!(t.next_event().await, None);
        assert_eq!(t.next_event().await, None);
        assert_eq!(t.next_event().await, None);
        assert_eq!(t.next_event().await, Some(Event::Delta("late text ".into())));
    }

    #[tokio::test]
    async fn contradictory_batch_rejected() {
        let page = ScriptPage::new(
            vec![StreamBatch { events: vec![Event::Delta("x ".into())], raced: true, aborted: true, finished: false, tool_calls: Vec::new() }],
            100.0,
        );
        let mut t = BridgeStreamTransport::new(page, Dialect::Chat);
        t.stream_reply(&serde_json::json!({})).await.unwrap();
        assert_eq!(t.next_event().await, None);
        assert!(t.failed.as_deref().unwrap_or("").contains("protocol violation"));
    }

    #[tokio::test]
    async fn stop_after_reaches_open() {
        // The stop MUST be baked into open_stream: the page aborts itself
        // in-loop, so the terminal is never read. Assert the plumbing, not
        // just the default.
        use std::sync::Mutex as StdMutex;
        struct RecStop {
            got: StdMutex<Vec<u32>>,
        }
        #[async_trait]
        impl PageStream for RecStop {
            async fn open_stream(&mut self, _: &serde_json::Value, _: Dialect, stop_after: u32) -> Result<String, String> {
                self.got.lock().unwrap().push(stop_after);
                Ok("s".into())
            }
            async fn drain(&mut self, _: &str) -> Result<StreamBatch, String> {
                Ok(StreamBatch { events: vec![], raced: false, aborted: false, finished: false, tool_calls: Vec::new() })
            }
            async fn cancel(&mut self, _: &str) -> Result<bool, String> {
                Ok(true)
            }
            async fn credits(&self) -> Result<f64, String> {
                Ok(1.0)
            }
        }
        let mut t = BridgeStreamTransport::new(RecStop { got: StdMutex::new(Vec::new()) }, Dialect::Chat);
        t.set_stop_after(200);
        t.stream_reply(&serde_json::json!({})).await.unwrap();
        assert_eq!(*t.page.got.lock().unwrap(), vec![200]);
    }

    #[tokio::test]
    async fn unwired_page_fails_closed() {
        let mut page = ScriptPage::new(vec![], 100.0);
        page.fail_open = true;
        let mut t = BridgeStreamTransport::new(page, Dialect::Responses);
        assert_eq!(
            t.stream_reply(&serde_json::json!({})).await,
            Err("live page stream not wired".into())
        );
    }

    #[tokio::test]
    async fn aborted_empty_batch_closes_stream() {
        // Regression: the page stops itself at the stop (aborted, no
        // terminal ever arrives BY DESIGN). An empty aborted batch yields
        // the explicit Aborted terminal (not None): the rung ends the leg
        // cleanly (raced=false) instead of polling a finished stream into
        // the stall bound (600s hang).
        let page = ScriptPage::new(
            vec![
                batch(&["hello "], false),
                StreamBatch { events: vec![], raced: false, aborted: true, finished: false, tool_calls: Vec::new() },
            ],
            100.0,
        );
        let mut t = BridgeStreamTransport::new(page, Dialect::Chat);
        t.stream_reply(&serde_json::json!({})).await.unwrap();
        assert_eq!(t.next_event().await, Some(Event::Delta("hello ".into())));
        assert_eq!(t.next_event().await, Some(Event::Aborted));
        assert_eq!(t.next_event().await, None);
        assert!(t.failed.is_none(), "aborted close is clean: {:?}", t.failed);
    }

    #[tokio::test]
    async fn stopped_text_batch_flows_then_aborts() {
        // THE live shape: page stopped itself at the stop WITH a final text
        // slice (aborted=true, events non-empty, no Done). The batch text
        // flows, then Aborted ends the stream (no 240-poll starvation).
        let page = ScriptPage::new(
            vec![batch_full(&["final text "], false, true)],
            100.0,
        );
        let mut t = BridgeStreamTransport::new(page, Dialect::Chat);
        t.stream_reply(&serde_json::json!({})).await.unwrap();
        assert_eq!(
            t.next_event().await,
            Some(Event::Delta("final text ".into()))
        );
        assert_eq!(t.next_event().await, Some(Event::Aborted));
        assert_eq!(t.next_event().await, None);
        assert!(t.failed.is_none(), "clean end, not failure: {:?}", t.failed);
    }

    #[tokio::test]
    async fn cancel_reaches_page() {
        let page = ScriptPage::new(vec![batch(&["a "], false)], 100.0);
        let mut t = BridgeStreamTransport::new(page, Dialect::Messages);
        t.stream_reply(&serde_json::json!({})).await.unwrap();
        t.next_event().await;
        t.cancel().await;
        assert_eq!(t.page.cancels, 1);
    }
}
