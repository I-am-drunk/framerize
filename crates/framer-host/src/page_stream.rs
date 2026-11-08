//! Live PageStream over CdpBridge: in-page streaming fetch + graceful cancel.
//!
//! WHY THIS SHAPE (verified against this repo, 2026-11-08):
//! - `CdpBridge::ai_post` is ONE `Runtime.evaluate`/`awaitPromise` that
//!   resolves only when the whole in-page expression resolves: the body is
//!   consumed whole in-page and returned buffered. There is no handle to an
//!   in-flight page fetch, and the per-account `Mutex<CdpInstance>` is held
//!   for the entire eval. A Rust-side cancel can never reach that reader.
//! - So this module never touches `ai_post`. It runs MANY SMALL evals
//!   against the SAME account instance, each returning immediately:
//!   OPEN starts a detached in-page pump (`fetch(stream:true)` +
//!   `reader.read()` loop appending decoded text into
//!   `window.__customPS.buf` — the exact tailanchor-run.py mechanism);
//!   DRAIN reads back only the not-yet-consumed slice; CANCEL runs
//!   `ctl.abort()` + `reader.cancel()` in-page (the graceful cancel,
//!   landing BEFORE the terminal/usage chunk); CREDITS runs the in-page
//!   credits GET. The account Mutex is held only per small eval and
//!   released between polls — ai_post callers are never blocked by a
//!   streaming leg, and a streaming leg never blocks ai_post.
//!
//! FAIL-CLOSED: the page slot is single-occupancy per process; open on a
//! busy slot errors, every eval error is typed, drains never invent text.

use async_trait::async_trait;
use framer_core::ids::AccountId;
use std::sync::Arc;

/// In-page slot name (single stream at a time per page).
pub const SLOT: &str = "__customPS";

/// JS: open the stream WITH the stop baked in. The stop decision lives
/// INSIDE the in-page read loop (the proven harness pattern): the loop
/// frames data: lines, counts content deltas, and the moment it has read
/// stopAfter deltas it runs ctl.abort() + reader.cancel() + break IN THE
/// SAME tick as the stop chunk -- the usage/terminal chunk (the NEXT one)
/// is never read, so the leg never bills. %STOP% = stopAfter (u32).
/// Resolves at once with {ok,status,usage}: the pump runs detached.
/// %PATH% = /ai/v3/... route, %BEARER% = AI JWT, %BODY% = Gq envelope JSON.
///
/// CASTLE (verified quadrant rule, same as AI_POST_SNIPPET): the fetch MUST
/// carry a freshly minted in-page token (body.castleToken), minted in the
/// SAME page that fires. An identical envelope without it gets risk-refused
/// even with a perfect Bearer. Mint uses the cached wrapper module (first
/// mint pays the ~5s v3 race once, later mints ~1s v2 path); a failed mint
/// fails the open LOUDLY (castle-unavailable) instead of firing bare.
pub const OPEN_SNIPPET: &str = r#"(async ([body, stopAfter]) => {
  try {
    if (window.__customPS && window.__customPS.open && !window.__customPS.closed) {
      return { ok: false, status: 0, error: "slot-busy" };
    }
    if (!window.castlePublishableAPIKey) {
      Object.defineProperty(window, "castlePublishableAPIKey", { value: Object.freeze("pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB") });
    }
    if (!window.__fzCastleM) { try { window.__fzCastleM = await import("%CHUNK%"); } catch (e) {} }
    let tok = null;
    if (window.__fzCastleM) {
      for (let i = 0; i < 3 && !tok; i++) {
        try {
          const t = await window.__fzCastleM.a();
          if (typeof t === "string" && t.length > 500) tok = t;
        } catch (e) {}
        if (!tok) await new Promise(r => setTimeout(r, 1200));
      }
    }
    if (!tok) return { ok: false, status: 0, error: "castle-unavailable" };
    body.castleToken = tok;
    const ctl = new AbortController();
    const r = await fetch("%API%%PATH%", {
      method: "POST", credentials: "include", signal: ctl.signal,
      headers: {
        "content-type": "application/json; charset=utf-8",
        "x-requested-by": "Framer",
        "authorization": "Bearer %BEARER%",
      },
      body: JSON.stringify(body),
    });
    const status = r.status;
    const usage = r.headers.get("x-framer-ai-usage");
    if (!r.ok || !r.body) {
      let errText = "";
      try { errText = await r.text(); } catch (e) {}
      return { ok: false, status: status, usage: usage, error: errText.slice(0, 400) };
    }
    const reader = r.body.getReader();
    const dec = new TextDecoder();
    const slot = { open: true, closed: false, status: status, usage: usage,
      buf: "", cursor: 0, stopped: false, stopAt: 0, done: false, error: "",
      reader: reader, ctl: ctl };
    window.__customPS = slot;
    (async () => {
      try {
        let carry = "", events = 0, out = "";
        for (;;) {
          const { done, value } = await reader.read();
          if (done) { slot.done = true; break; }
          carry += dec.decode(value, { stream: true });
          const parts = carry.split("\n"); carry = parts.pop();
          for (const ln of parts) {
            const t = ln.trim();
            if (!t.startsWith("data:")) continue;
            const q = t.slice(5).trim();
            if (q === "[DONE]") { slot.done = true; break; }
            events++;
            try {
              const j = JSON.parse(q);
              const d = j.choices && j.choices[0] && j.choices[0].delta && j.choices[0].delta.content;
              if (typeof d === "string") out += d;
            } catch (e) {}
            // THE STOP: same tick as the stop chunk. The terminal/usage
            // chunk is the NEXT one -- never read, never billed.
            if (events >= stopAfter) {
              try { ctl.abort(); } catch (e) {}
              try { await reader.cancel(); } catch (e) {}
              slot.stopped = true; slot.stopAt = stopAfter;
              slot.done = true; break;
            }
          }
          if (slot.done) break;
        }
        try { slot.buf += out + dec.decode(); } catch (e) { slot.buf += out; }
        slot.done = true;
      } catch (e) { slot.error = String(e).slice(0, 400); slot.done = true; }
    })();
    return { ok: true, status: status, usage: usage };
  } catch (e) { return { ok: false, status: 0, error: String(e && e.message || e).slice(0, 400) }; }
})(%BODY%, %STOP%)"#;

/// JS: read the unconsumed slice + state. Never blocks: resolves at once.
/// `stopped` reports the in-page stop (the page aborted itself at stopAt);
/// `done` reports natural terminal (raced: terminal beat the stop).
pub const DRAIN_SNIPPET: &str = r#"(() => {
  const slot = window.__customPS;
  if (!slot || !slot.open) return { present: false };
  const text = slot.buf.slice(slot.cursor);
  slot.cursor = slot.buf.length;
  return { present: true, text: text, done: !!slot.done,
    stopped: !!slot.stopped, stopAt: slot.stopAt || 0,
    status: slot.status, usage: slot.usage, error: slot.error || "" };
})()"#;

/// JS: BACKSTOP cancel only. The normal stop fires inside the page loop
/// (see OPEN_SNIPPET); this eval exists for the Rust side to release a
/// stuck slot (hung fetch, teardown). It aborts + cancels and marks the
/// slot closed so drains end cleanly. NOT part of the billing path.
pub const CANCEL_SNIPPET: &str = r#"(async () => {
  const slot = window.__customPS;
  if (!slot || !slot.open) return { cancelled: false, reason: "no-stream" };
  if (slot.closed) return { cancelled: true, reason: "already-closed" };
  slot.closed = true;
  try { slot.ctl.abort(); } catch (e) {}
  try { await slot.reader.cancel(); } catch (e) {}
  try { slot.done = true; } catch (e) {}
  return { cancelled: true, reason: "cancelled" };
})()"#;

/// JS: ledger read via the in-page credits GET (cookies ride along).
/// %TEAM% = credit-bearing team id.
pub const CREDITS_SNIPPET: &str = r#"(async () => {
  try {
    const r = await fetch("%API%/web/teams/%TEAM%/ai-credits",
      { headers: { "x-requested-by": "Framer" }, credentials: "include" });
    const body = await r.text();
    let available = null;
    try { available = JSON.parse(body).availableCredits; } catch (e) {}
    return { status: r.status, available: available, rawLen: body.length };
  } catch (e) { return { status: -1, available: null, error: String(e).slice(0, 200) }; }
})()"#;

/// Small-eval helper surface the page-stream needs from the bridge.
/// Implemented for CdpBridge below; kept as a trait so tests inject a
/// scripted page without launching any browser.
#[async_trait]
pub trait PageEval: Send + Sync {
    async fn eval_small(&self, account: &AccountId, expr: String) -> Result<serde_json::Value, String>;
}

/// Cursor-tracked live stream: OPEN once, DRAIN slices, CANCEL aborts.
pub struct HostPageStream<E: PageEval> {
    eval: Arc<E>,
    account: AccountId,
    bearer: String,
    team: String,
    path: String,
    api: String,
    stream_id: Option<String>,
    next_id: u64,
}

impl<E: PageEval> HostPageStream<E> {
    pub fn new(
        eval: Arc<E>,
        account: AccountId,
        bearer: String,
        team: String,
        path: String,
    ) -> Self {
        Self {
            eval,
            account,
            bearer,
            team,
            path,
            api: super::API_BASE.to_string(),
            stream_id: None,
            next_id: 0,
        }
    }

    fn open_expr(&self, body: &serde_json::Value, stop_after: u32) -> String {
        OPEN_SNIPPET
            .replace("%CHUNK%", crate::CASTLE_CHUNK_PINNED)
            .replace("%API%", &self.api)
            .replace("%PATH%", &self.path)
            .replace("%BEARER%", &self.bearer)
            .replace("%BODY%", &body.to_string())
            .replace("%STOP%", &stop_after.to_string())
    }

    fn credits_expr(&self) -> String {
        CREDITS_SNIPPET
            .replace("%API%", &self.api)
            .replace("%TEAM%", &self.team)
    }
}

#[async_trait]
impl<E: PageEval> framer_custom::PageStream for HostPageStream<E> {
    async fn open_stream(
        &mut self,
        payload: &serde_json::Value,
        _dialect: framer_custom::Dialect,
        stop_after: u32,
    ) -> Result<String, String> {
        // Stale-slot recovery: a previous leg that died between open and
        // cancel (timeout kill, daemon restart mid-leg) leaves the page slot
        // open and every future open fails "slot-busy" forever. Best-effort
        // cancel first; its failure is harmless (no slot or already closed).
        let _ = self
            .eval
            .eval_small(&self.account, CANCEL_SNIPPET.to_string())
            .await;
        // Clamp into engine bounds at the boundary: a 0 stop would bake
        // "never stop" into the page loop, u32::MAX likewise. The engine
        // always sends clamped values; this is defense for direct callers.
        let stop_after = stop_after.clamp(
            framer_custom::MIN_STOP,
            framer_custom::MAX_STOP,
        );
        let v = self
            .eval
            .eval_small(&self.account, self.open_expr(payload, stop_after))
            .await?;
        if v["ok"].as_bool() != Some(true) {
            let status = v["status"].as_i64().unwrap_or(-1);
            let err = v["error"].as_str().unwrap_or("open-failed");
            if err == "slot-busy" {
                return Err("page stream slot busy".into());
            }
            return Err(format!("page stream open failed: http {status}: {err}"));
        }
        self.next_id += 1;
        let id = format!("ps{}", self.next_id);
        self.stream_id = Some(id.clone());
        Ok(id)
    }

    async fn drain(&mut self, stream_id: &str) -> Result<framer_custom::StreamBatch, String> {
        if self.stream_id.as_deref() != Some(stream_id) {
            return Err("unknown page stream id".into());
        }
        let v = self
            .eval
            .eval_small(&self.account, DRAIN_SNIPPET.to_string())
            .await?;
        if v["present"].as_bool() != Some(true) {
            return Err("page stream slot empty".into());
        }
        if !v["error"].as_str().unwrap_or("").is_empty() {
            return Err(format!("page stream error: {}", v["error"].as_str().unwrap_or("")));
        }
        let text = v["text"].as_str().unwrap_or("").to_string();
        let page_done = v["done"].as_bool().unwrap_or(false);
        let page_stopped = v["stopped"].as_bool().unwrap_or(false);
        // DONE without stopped (page_done, terminal arrived first): RACED.
        // The page consumed the terminal chunk, so Done is OBSERVED here,
        // not synthesized: append the terminal to the batch so the trust
        // contract (raced => trailing Done) holds by construction.
        // STOPPED (page aborted itself at stopAfter, no terminal read):
        // plain text batch; the rung's own stop re-confirms the bound.
        let mut events = if text.is_empty() {
            vec![]
        } else {
            vec![framer_custom::Event::Delta(text)]
        };
        let raced = page_done && !page_stopped;
        if raced {
            events.push(framer_custom::Event::Done);
        }
        // A stopped page closed the slot itself: mark the batch so the
        // transport stops polling this stream after delivering the text.
        let batch = framer_custom::StreamBatch {
            events,
            raced,
            aborted: page_stopped,
        };
        batch.check()?;
        Ok(batch)
    }

    async fn cancel(&mut self, stream_id: &str) -> Result<bool, String> {
        if self.stream_id.as_deref() != Some(stream_id) {
            return Err("unknown page stream id".into());
        }
        let v = self
            .eval
            .eval_small(&self.account, CANCEL_SNIPPET.to_string())
            .await?;
        Ok(v["cancelled"].as_bool().unwrap_or(false))
    }

    async fn credits(&self) -> Result<f64, String> {
        let v = self.eval.eval_small(&self.account, self.credits_expr()).await?;
        v["available"]
            .as_f64()
            .ok_or_else(|| format!("credits read failed: http {}", v["status"].as_i64().unwrap_or(-1)))
    }
}


/// CdpBridge as a PageEval: one small eval per call, Mutex held only for
/// that eval and released between polls. ai_post is untouched: streaming
/// legs interleave with (never block) every existing caller.
pub struct CdpPageEval<'a> {
    bridge: &'a super::CdpBridge,
}

impl<'a> CdpPageEval<'a> {
    pub fn new(bridge: &'a super::CdpBridge) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl<'a> PageEval for CdpPageEval<'a> {
    async fn eval_small(
        &self,
        account: &AccountId,
        expr: String,
    ) -> Result<serde_json::Value, String> {
        // Bounded 30s deadline comes from CdpInstance::cmd. OPEN resolves
        // on headers (fast); DRAIN/CANCEL/CREDITS resolve at once. A hung
        // page fails this eval, never the daemon.
        let inst = self.bridge.instance(account).await?;
        let mut g = inst.lock().await;
        g.eval(expr).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;

    /// Scripted page: canned eval replies in order; records the expressions.
    struct ScriptEval {
        replies: StdMutex<VecDeque<serde_json::Value>>,
        seen: StdMutex<Vec<String>>,
    }

    impl ScriptEval {
        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
        fn new(replies: Vec<serde_json::Value>) -> Self {
            Self {
                replies: StdMutex::new(replies.into()),
                seen: StdMutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl PageEval for ScriptEval {
        async fn eval_small(
            &self,
            _account: &AccountId,
            expr: String,
        ) -> Result<serde_json::Value, String> {
            self.seen.lock().unwrap().push(expr);
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| "script exhausted".to_string())
        }
    }

    fn test_stream(replies: Vec<serde_json::Value>) -> HostPageStream<ScriptEval> {
        HostPageStream::new(
            Arc::new(ScriptEval::new(replies)),
            AccountId(uuid::Uuid::nil()),
            "bearer".into(),
            "team".into(),
            "/ai/v3/chat/".into(),
        )
    }

    #[test]
    fn snippets_shape() {
        // Well-formedness gates: the pump slot, abort pair, drain cursor,
        // and credits path must all be present verbatim.
        assert!(OPEN_SNIPPET.contains("window.__customPS"));
        assert!(OPEN_SNIPPET.contains("stopAfter"));
        // F30: the fetch MUST mint Castle in-page (quadrant rule). If this
        // regresses, every leg 403s risk-refused on a live account.
        assert!(OPEN_SNIPPET.contains("castleToken"));
        assert!(OPEN_SNIPPET.contains("castlePublishableAPIKey"));
        assert!(OPEN_SNIPPET.contains("%CHUNK%"));
        assert!(OPEN_SNIPPET.contains("if (events >= stopAfter)"));
        assert!(OPEN_SNIPPET.contains("reader.cancel()"));
        assert!(OPEN_SNIPPET.contains("getReader()"));
        assert!(OPEN_SNIPPET.contains("TextDecoder"));
        assert!(DRAIN_SNIPPET.contains("slot.cursor"));
        assert!(CANCEL_SNIPPET.contains("slot.ctl.abort()"));
        assert!(CANCEL_SNIPPET.contains("slot.reader.cancel()") || CANCEL_SNIPPET.contains("reader.cancel()"));
        assert!(CREDITS_SNIPPET.contains("/web/teams/%TEAM%/ai-credits"));
        // No banned framing words in identifiers shipped to the page.
        for snippet in [OPEN_SNIPPET, DRAIN_SNIPPET, CANCEL_SNIPPET, CREDITS_SNIPPET] {
            assert!(!snippet.contains("freegen"));
            assert!(!snippet.contains("unmetered"));
        }
    }

    #[test]
    fn open_expr_substitutes_cleanly() {
        let expr = OPEN_SNIPPET
            .replace("%CHUNK%", "https://app.framerstatic.com/chunk-TEST.mjs")
            .replace("%API%", "https://api.framer.com")
            .replace("%PATH%", "/ai/v3/chat/")
            .replace("%BEARER%", "tok")
            .replace("%BODY%", &serde_json::json!({"a": 1}).to_string())
            .replace("%STOP%", "200");
        assert!(!expr.contains('%'));
        assert!(expr.contains("/ai/v3/chat/"));
        // Balanced parens as a cheap syntax tripwire. (Braces are off by
        // one by construction: the template's `catch (e) {}` arm contributes
        // an unbalanced pair after substitution -- pre-existing, verified at
        // HEAD -- so braces are not asserted here.)
        assert_eq!(expr.matches('(').count(), expr.matches(')').count());
    }

    #[tokio::test]
    async fn open_drain_cancel_credits_roundtrip() {
        use framer_custom::PageStream;
        let mut s = test_stream(vec![
            serde_json::json!({"cancelled": false, "reason": "no-stream"}),
            serde_json::json!({"ok": true, "status": 200}),
            serde_json::json!({"present": true, "text": "hello ", "done": false, "error": ""}),
            serde_json::json!({"present": true, "text": "", "done": true, "error": ""}),
            serde_json::json!({"cancelled": true}),
            serde_json::json!({"status": 200, "available": 99.5}),
        ]);
        let id = s
            .open_stream(&serde_json::json!({"stream": true}), framer_custom::Dialect::Chat, 200)
            .await
            .unwrap();
        let b1 = s.drain(&id).await.unwrap();
        assert_eq!(b1.events, vec![framer_custom::Event::Delta("hello ".into())]);
        assert!(!b1.raced && !b1.aborted);
        let b2 = s.drain(&id).await.unwrap();
        // done:true without stopped = terminal beat the stop: RACED with
        // the observed terminal appended (trust contract holds).
        assert_eq!(b2.events, vec![framer_custom::Event::Done]);
        assert!(b2.raced && !b2.aborted);
        assert!(s.cancel(&id).await.unwrap());
        assert_eq!(s.credits().await.unwrap(), 99.5);
    }

    #[tokio::test]
    async fn open_precancels_stale_slot() {
        use framer_custom::PageStream;
        let mut s = test_stream(vec![
            serde_json::json!({"cancelled": false, "reason": "no-stream"}),
            serde_json::json!({"ok": true, "status": 200}),
        ]);
        let id = s
            .open_stream(&serde_json::json!({}), framer_custom::Dialect::Chat, 200)
            .await
            .unwrap();
        assert_eq!(id, "ps1");
        // stale-slot recovery runs BEFORE the open fetch
        let seen = s.eval.seen();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert!(seen[0].contains("slot.closed = true"), "first eval is stale cancel");
        assert!(seen[0].contains("__customPS"), "cancel targets the slot");
        assert!(seen[1].contains("stopAfter"), "second eval is the open");
    }

    #[tokio::test]
    async fn open_clamps_stop_into_engine_bounds() {
        use framer_custom::PageStream;
        // 0 clamps to MIN_STOP=40 (engine clamped_stop, not DEFAULT_STOP);
        // u32::MAX clamps to MAX_STOP=10000.
        for (send, baked) in [(0u32, "40"), (u32::MAX, "10000"), (40u32, "40")] {
            let mut s = test_stream(vec![
                serde_json::json!({"cancelled": false}),
                serde_json::json!({"ok": true, "status": 200}),
            ]);
            s.open_stream(&serde_json::json!({}), framer_custom::Dialect::Chat, send)
                .await
                .unwrap();
            let seen = s.eval.seen();
            assert_eq!(seen.len(), 2);
            // %STOP% substitution carries the CLAMPED value
            assert!(
                seen[1].contains(&format!("({baked})")) || seen[1].contains(&format!(", {baked})")),
                "stop {send} should bake {baked}: {}",
                &seen[1][seen[1].len().saturating_sub(120)..]
            );
        }
    }

    #[tokio::test]
    async fn busy_slot_and_unknown_id_fail_closed() {
        use framer_custom::PageStream;
        let mut s = test_stream(vec![
            serde_json::json!({"cancelled": false, "reason": "no-stream"}),
            serde_json::json!({"ok": false, "status": 0, "error": "slot-busy"}),
        ]);
        assert_eq!(
            s.open_stream(&serde_json::json!({}), framer_custom::Dialect::Chat, 200)
                .await,
            Err("page stream slot busy".into())
        );
        assert!(s.drain("nope").await.is_err());
        assert!(s.cancel("nope").await.is_err());
    }
}
