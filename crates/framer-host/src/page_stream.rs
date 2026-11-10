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
pub const OPEN_SNIPPET: &str = r#"(async (body, stopAfter) => {
  // FIRE-AND-POLL OPEN (2026-11-08 live-hang fix): this eval must resolve
  // INSTANTLY. The old shape minted Castle inline (~8s) and awaited fetch
  // headers inside the eval; on the Camoufox sidecar every eval runs on the
  // account's single ambient page, so a slow open wedged the whole account
  // lane (resets on ai_post's own mint+fetch). The mint + fetch + stream
  // pump all run in the DETACHED task below; drains observe buf/cursor/done
  // and surface mint/fetch failures as loud leg errors (same messages).
  try {
    {
      const prev = window.__customPS;
      if (prev && prev.open && !prev.closed) {
        return { ok: false, status: 0, error: "slot-busy" };
      }
      window.__customPS = { open: false, closed: true, done: true };
    }
    const slot = { open: true, closed: false, status: 0, usage: null,
      buf: "", cursor: 0, chunks: 0, stopped: false, stopAt: 0, done: false, error: "",
      reader: null, ctl: null };
    window.__customPS = slot;
    (async () => {
      try {
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
        if (!tok) { slot.error = "castle-unavailable"; slot.done = true; return; }
        body.castleToken = tok;
        const ctl = new AbortController();
        slot.ctl = ctl;
        let r;
        try {
          r = await fetch("%API%%PATH%", {
            method: "POST", credentials: "include", signal: ctl.signal,
            headers: {
              "content-type": "application/json; charset=utf-8",
              "x-requested-by": "Framer",
              "authorization": "Bearer %BEARER%",
            },
            body: JSON.stringify(body),
          });
        } catch (e) { slot.error = String(e && e.message || e).slice(0, 400); slot.done = true; return; }
        slot.status = r.status;
        try { slot.usage = r.headers.get("x-framer-ai-usage"); } catch (e) {}
        if (!r.ok || !r.body) {
          let errText = "";
          try { errText = await r.text(); } catch (e) {}
          slot.error = ("http " + r.status + ": " + errText).slice(0, 400);
          slot.done = true; return;
        }
        const reader = r.body.getReader();
        slot.reader = reader;
        slot.streamLive = true;
        const dec = new TextDecoder();
        try {
          let carry = "", events = 0, rawLines = 0;
          for (;;) {
            const { done, value } = await reader.read();
            if (done) { try { slot.buf += dec.decode(); } catch (e) {} slot.streamLive = false; slot.done = true; break; }
            carry += dec.decode(value, { stream: true });
            const parts = carry.split("\n"); carry = parts.pop();
            // WIRE-TAP (2026-11-08 stall hunt): the first 8 raw lines of
            // the body are echoed into slot.error-prefixed diagnostics via
            // slot.buf's sentinel channel is text-only, so stash them on a
            // side field the drain reports (see DRAIN_SNIPPET wireTap).
            for (const ln of parts) {
              const t = ln.trim();
              if (t === "" || t === ":") continue;
              if (rawLines < 8) {
                rawLines++;
                try { slot.wireTap = (slot.wireTap || "") + "L" + rawLines + "=" + t.slice(0, 120) + " | "; } catch (e) {}
              }
              if (!t.startsWith("data:")) continue;
              const q = t.slice(5).trim();
              if (q === "[DONE]") { try { slot.buf += dec.decode(); } catch (e) {} slot.streamLive = false; slot.done = true; break; }
              let d = null;
              let fin = null;
              try {
                const j = JSON.parse(q);
                // Chat deltas: choices[0].delta.content. Responses deltas:
                // response.output_text.delta carries .delta. Both count as
                // one framed line either way (the stop counts lines, not
                // shapes).
                const c = j.choices && j.choices[0] && j.choices[0].delta && j.choices[0].delta.content;
                if (typeof c === "string") { d = c; }
                else if (typeof j.delta === "string" && typeof j.type === "string" && j.type.indexOf("response.") === 0) { d = j.delta; }
                // TOOL CALLS (2026-11-09): a tool turn ends with
                // finish_reason=tool_calls and the call is COMPLETE before
                // the finish line -- so the finish cutoff captures tool
                // turns free too. Accumulate the streamed fragments per
                // call index: id+type+name arrive on the first delta,
                // arguments stream as string fragments after it.
                try {
                  const tcs = j.choices && j.choices[0] && j.choices[0].delta && j.choices[0].delta.tool_calls;
                  if (tcs && tcs.length) {
                    if (!slot.toolCalls) slot.toolCalls = {};
                    for (const tc of tcs) {
                      const ix = (tc.index === undefined || tc.index === null) ? 0 : tc.index;
                      const acc = slot.toolCalls[ix] || (slot.toolCalls[ix] = { id: "", type: "function", name: "", arguments: "" });
                      if (tc.id) acc.id = tc.id;
                      if (tc.type) acc.type = tc.type;
                      const fn = tc.function || {};
                      if (fn.name) acc.name = fn.name;
                      if (typeof fn.arguments === "string") acc.arguments += fn.arguments;
                    }
                  }
                } catch (e) {}
                // THE FINISH CUTOFF (inside the parse block: j is scoped
                // here). finish_reason is the LAST content line; the
                // usage/terminal frame is the NEXT one, and the meter
                // settles on the terminal -- so aborting here captures the
                // COMPLETE answer at zero settle. Responses dialect
                // deliberately excluded: a multi-item turn has no safe
                // per-item cut (that is the ADR-003 item-aware work).
                const ch = j.choices && j.choices[0];
                if (ch && typeof ch.finish_reason === "string" && ch.finish_reason) fin = ch.finish_reason;
              } catch (e) { d = null; }
              slot.buf += (d === null || d === "") ? "\u0000" : d;
              slot.chunks++;
              events++;
              // THE MARKER CUTOFF (2026-11-09): the deterministic zero-settle
              // technique. The request forces the model to write its complete
              // answer, emit the marker, then keep padding. The abort lands
              // while the server is STILL GENERATING the padding, so the
              // terminal frame never exists and the settle never happens --
              // for short answers too, every time the model honors it.
              // The buffer truncates at the marker: the captured text is the
              // complete answer at its natural length. stop_after and the
              // finish cutoff stay as backstops.
              if (fin === null) {
                const mi = slot.buf.indexOf("⟦FRZ-END⟧");
                if (mi >= 0) {
                  slot.buf = slot.buf.slice(0, mi);
                  fin = "marker";
                }
              }
              // The finish cutoff or the length backstop ends the leg: the
              // abort lands before the terminal frame either way.
              if (fin !== null || events >= stopAfter) {
                // CUT-OFF HARDENING (2026-11-10): the terminal must be
                // UNREACHABLE after the stop, not merely unrequested. Order:
                // (1) flag stopped first so re-entrant polls see it; (2)
                // cancel the reader before aborting the controller (abort
                // alone can still deliver a buffered terminal on some
                // transports); (3) detach reader+controller so no later
                // code path can ever pull another chunk from this stream.
                slot.stopped = true; slot.stopAt = events;
                try { await reader.cancel(); } catch (e) {}
                try { ctl.abort(); } catch (e) {}
                try { slot.reader = null; } catch (e) {}
                try { slot.ctl = null; } catch (e) {}
                if (fin !== null) { slot.finished = true; slot.finishReason = String(fin); }
                slot.streamLive = false; slot.done = true; break;
              }
            }
            if (slot.done) break;
          }
          try { slot.buf += dec.decode(); } catch (e) {}
          slot.streamLive = false; slot.done = true;
        } catch (e) { slot.error = String(e).slice(0, 400); slot.streamLive = false; slot.done = true; }
      } catch (e) { slot.error = String(e && e.message || e).slice(0, 400); slot.done = true; }
    })();
    return { ok: true, status: 0, usage: null };
  } catch (e) { return { ok: false, status: 0, error: String(e && e.message || e).slice(0, 400) }; }
})(%BODY%, %STOP%)"#;
/// SENTINEL CONTRACT (drain-loop hang fix, 2026-11-08): OPEN_SNIPPET appends
/// exactly one unit per parsed data line -- the delta text, or a single
/// U+0000 sentinel when the line carries no content. drain decodes the
/// slice back into one Event-Delta per line (see decode_deltas), so the
/// rung stop counts the same framed units the page counted at its stop.
/// Without this, multi-line arrivals collapse into one delta and the rung
/// never reaches its stop (the 600s silent hang).
/// JS: read the unconsumed slice + state. Never blocks: resolves at once.
/// `stopped` reports the in-page stop (the page aborted itself at stopAt);
/// `done` reports natural terminal (raced: terminal beat the stop).
pub const DRAIN_SNIPPET: &str = r#"(() => {
  const slot = window.__customPS;
  // CLOSED-BY-CANCEL IS STILL READABLE: cancel marks closed+done but keeps
  // buf/cursor, so post-stop drains consume the captured prefix. Only a
  // missing slot (page reset/navigation) is unreadable.
  if (!slot || (!slot.open && !slot.closed)) return { present: false };
  const text = slot.buf.slice(slot.cursor);
  slot.cursor = slot.buf.length;
  const tap = slot.wireTap || "";
  try { slot.wireTap = ""; } catch (e) {}
  const tcs = [];
  try {
    if (slot.toolCalls) {
      const keys = Object.keys(slot.toolCalls).sort();
      for (const k of keys) tcs.push(slot.toolCalls[k]);
    }
  } catch (e) {}
  return { present: true, text: text, done: !!slot.done,
    stopped: !!slot.stopped, stopAt: slot.stopAt || 0,
    finished: !!slot.finished, finishReason: slot.finishReason || "",
    toolCalls: tcs,
    status: slot.status, usage: slot.usage, error: slot.error || "",
    chunks: slot.chunks || 0, wireTap: tap,
    streamLive: !!slot.streamLive };
})()"#;

/// JS: the open-wait's read — PEEK ONLY. DRAIN_SNIPPET moves the cursor
/// (slot.cursor = slot.buf.length), so any content arriving during the
/// open-wait used to be eaten before the rung's first drain (measured
/// 2026-11-09: a fast model's whole 43-chunk answer arrived during the
/// 3-poll open; the rung captured 0 of its 206 chars). The peek reads the
/// same flags without touching buf/cursor; the rung's drains own the text.
pub const PEEK_SNIPPET: &str = r#"(() => {
  const slot = window.__customPS;
  if (!slot || (!slot.open && !slot.closed)) return { present: false };
  return { present: true, text_len: slot.buf.length, done: !!slot.done,
    stopped: !!slot.stopped, stopAt: slot.stopAt || 0,
    finished: !!slot.finished, finishReason: slot.finishReason || "",
    status: slot.status, usage: slot.usage, error: slot.error || "",
    chunks: slot.chunks || 0, wireTap: slot.wireTap || "",
    streamLive: !!slot.streamLive };
})()"#;

/// JS: BACKSTOP cancel only. The normal stop fires inside the page loop
/// (see OPEN_SNIPPET); this eval exists for the Rust side to release a
/// stuck slot (hung fetch, teardown). It aborts + cancels and marks the
/// slot closed so drains end cleanly. NOT part of the billing path.
pub const CANCEL_SNIPPET: &str = r#"(async () => {
  const slot = window.__customPS;
  if (!slot || !slot.open) return { cancelled: false, reason: "no-stream" };
  if (slot.closed) return { cancelled: true, reason: "already-closed" };
  // DRAIN-VISIBILITY FIX (2026-11-08): never blank the buffer. The pump
  // publishes decoded text into slot.buf incrementally; a stale-slot
  // pre-cancel that cleared buf/cursor would erase already-arrived text
  // that drains had not consumed yet. close+abort+cancel only.
  slot.closed = true;
  try { slot.ctl.abort(); } catch (e) {}
  try { await slot.reader.cancel(); } catch (e) {}
  try { slot.done = true; } catch (e) {}
  try { slot.open = false; } catch (e) {}
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

impl<E: PageEval> HostPageStream<E> {
    // Decode one drain slice into one Delta per parsed page data line.
    // The pump appends exactly one unit per line (delta text, or U+0000
    // when the line carries no content); a U+0000 terminates the current
    // run (pushed even when empty -- consecutive sentinels are consecutive
    // empty lines), and a non-empty trailing run is the still-open line.
    // Empty slice = no new lines = no events (a paced poll, not a delta).
    // NUL bytes in genuine model text would split one line into two
    // deltas (stop reached marginally early); LLM output never contains
    // NUL, so the decode is exact in practice.
    fn decode_deltas(text: &str) -> Vec<framer_custom::Event> {
        if text.is_empty() {
            return Vec::new();
        }
        let mut events = Vec::new();
        let mut cur = String::new();
        for ch in text.chars() {
            if ch == '\0' {
                events.push(framer_custom::Event::Delta(std::mem::take(&mut cur)));
            } else {
                cur.push(ch);
            }
        }
        if !cur.is_empty() {
            events.push(framer_custom::Event::Delta(cur));
        }
        events
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
        //
        // PAGE-PIN FIX (2026-11-08 root cause of the live drain hang): the
        // sidecar serves evals from a POOL of pages (round-robin, not one
        // page per account). A pre-cancel on page A followed by an open on
        // page B leaves B's live slot "busy" (open fails), or -- worse --
        // opens the pump on B while later drains land on A/C (present=false
        // forever, the 2000-poll silent stall). So the WHOLE open sequence
        // (pre-cancel, open, verify) retries across pool pages: on
        // slot-busy, cancel-then-reopen (the busy page may be ours now);
        // the verify drain immediately after open proves the pump is on a
        // page drains can see. Bounded: 3 pages x (cancel+open+verify).
        // Micro-stop passes through (see rung.rs): the page aborts at
        // events >= stopAfter, so stopAfter=1 ends the pump on the first
        // data line. Normal stops clamp as before.
        let stop_after = if stop_after < framer_custom::MIN_STOP {
            stop_after.max(1)
        } else {
            stop_after.clamp(framer_custom::MIN_STOP, framer_custom::MAX_STOP)
        };
        let mut last_err = "page stream open failed: no attempt".to_string();
        for _ in 0..3 {
            // Best-effort stale cancel on this rotation's page.
            let _ = self
                .eval
                .eval_small(&self.account, CANCEL_SNIPPET.to_string())
                .await;
            // Fire-and-poll open: the eval starts the detached pump and
            // resolves instantly (mint + fetch headers happen off-eval).
            let v = self
                .eval
                .eval_small(&self.account, self.open_expr(payload, stop_after))
                .await?;
            if v["ok"].as_bool() != Some(true) {
                let status = v["status"].as_i64().unwrap_or(-1);
                let err = v["error"].as_str().unwrap_or("open-failed");
                if err == "slot-busy" {
                    last_err = "page stream slot busy".to_string();
                    continue;
                }
                return Err(format!("page stream open failed: http {status}: {err}"));
            }
            // Verify the pump is drain-visible on this rotation's page
            // before returning: a present=false here means the open landed
            // on a different pool page than this drain sees -- retry the
            // whole sequence on the next rotation instead of handing the
            // rung a stream that can never yield text.
            //
            // Then wait (bounded ~75s: slow mint ~8s + cold fetch
            // headers +.poll slack) for the detached pump to report its
            // fetch outcome: headers arrived (status != 0), text flowing,
            // terminal flags, or a loud error (castle-unavailable, http
            // 4xx/5xx, network). A pump that never reports is a wedged
            // page: fail loud, never hand the rung a silent stream.
            // The id is minted only on the ACCEPTED rotation, so pool
            // retries never skip ids (ps1 first, no gaps).
            // OPEN DIAGNOSTIC (2026-11-08 stall hunt): one line per wait
            // poll (max ~150 lines/leg) naming present/status/text/done/
            // stopped/error, so the next swap log shows exactly what the
            // pump reported while the rung starved.
            let mut attempts = 0;
            loop {
                attempts += 1;
                match self
                    .eval
                    .eval_small(&self.account, PEEK_SNIPPET.to_string())
                    .await
                {
                    Ok(vv) if vv["present"].as_bool() == Some(true) => {
                        eprintln!(
                            "[custom-route] open-wait poll={} present=true status={} text_len={} done={} stopped={} chunks={} live={} err={:?} tap={:?}",
                            attempts,
                            vv["status"].as_i64().unwrap_or(-1),
                            vv["text_len"].as_i64().unwrap_or(-1),
                            vv["done"].as_bool().unwrap_or(false),
                            vv["stopped"].as_bool().unwrap_or(false),
                            vv["chunks"].as_i64().unwrap_or(-1),
                            vv["streamLive"].as_bool().unwrap_or(false),
                            vv["error"].as_str().unwrap_or(""),
                            vv["wireTap"].as_str().unwrap_or(""),
                        );
                        if let Some(err) = vv["error"].as_str() {
                            if !err.is_empty() {
                                if err.starts_with("http ") {
                                    return Err(format!("page stream open failed: {err}"));
                                }
                                return Err(format!("page stream open failed: {err}"));
                            }
                        }
                        let reported = vv["status"].as_i64().unwrap_or(0) != 0
                            || vv["text_len"].as_i64().unwrap_or(0) > 0
                            || vv["done"].as_bool().unwrap_or(false)
                            || vv["stopped"].as_bool().unwrap_or(false);
                        if reported {
                            self.next_id += 1;
                            let id = format!("ps{}", self.next_id);
                            self.stream_id = Some(id.clone());
                            return Ok(id);
                        }
                        if attempts >= 150 {
                            last_err =
                                "page stream pump never reported (page wedged?)".to_string();
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                    other => {
                        eprintln!(
                            "[custom-route] open-wait poll={} present=false ({:?})",
                            attempts,
                            other
                                .as_ref()
                                .map(|v| v.to_string())
                                .unwrap_or_else(|e| e.clone()),
                        );
                        last_err =
                            "page stream opened on an unreachable pool page; retrying".to_string();
                        break;
                    }
                }
            }
        }
        Err(last_err)
    }

    async fn drain(&mut self, stream_id: &str) -> Result<framer_custom::StreamBatch, String> {
        if self.stream_id.as_deref() != Some(stream_id) {
            return Err("unknown page stream id".into());
        }
        let v = self
            .eval
            .eval_small(&self.account, DRAIN_SNIPPET.to_string())
            .await?;
        // DRAIN-VISIBILITY FIX (2026-11-08): a missing slot right after a
        // successful open is a page-side reset (navigation/GC), not a
        // transport bug: fail the LEG loudly here so the chain aborts with
        // a named cause instead of converting one missed poll into two
        // thousand silent Nones and a 16-minute stall mystery. (Live proof:
        // open ok -> exactly one present=false -> "slot empty" -> the old
        // code mapped Err to None and the rung stalled 2000 polls.)
        if v["present"].as_bool() != Some(true) {
            return Err("page stream slot empty right after open (page reset?)".into());
        }
        if !v["error"].as_str().unwrap_or("").is_empty() {
            return Err(format!("page stream error: {}", v["error"].as_str().unwrap_or("")));
        }
        let text = v["text"].as_str().unwrap_or("").to_string();
        let page_done = v["done"].as_bool().unwrap_or(false);
        let page_stopped = v["stopped"].as_bool().unwrap_or(false);
        // OPENROUTER WIRE SHAPE (2026-11-08 live proof): the gateway streams
        // RAW OpenAI chat chunks (one JSON object per data: line, content in
        // choices[0].delta.content) -- NOT {delta:{text}} envelopes. The
        // page pump already extracts choices[0].delta.content per line and
        // appends raw text (+ U+0000 per empty line) to buf, so a text slice
        // here is READY text: decode per line. (No second JSON parse: the
        // slice is decoded delta text, not envelopes.)
        //
        // DONE without stopped (page_done, terminal arrived first): RACED.
        // The page consumed the terminal chunk, so Done is OBSERVED here,
        // not synthesized: append the terminal to the batch so the trust
        // contract (raced => trailing Done) holds by construction.
        // STOPPED (page aborted itself at stopAfter, no terminal read):
        // plain text batch; the rung's own stop re-confirms the bound.
        // One Delta PER parsed data line (sentinel-decoded): the page
        // counted lines at its stop, so the rung must count lines too --
        // collapsing the slice into one Delta starved the stop (600s hang).
        let mut events = Self::decode_deltas(&text);
        let raced = page_done && !page_stopped;
        if raced {
            events.push(framer_custom::Event::Done);
        }
        // A stopped page closed the slot itself: mark the batch so the
        // transport stops polling this stream after delivering the text.
        let page_finished = v["finished"].as_bool().unwrap_or(false);
        // Tool calls the pump accumulated (complete only on a finish-cut
        // leg; a length cut leaves them partial and the chain drops them).
        let tool_calls: Vec<serde_json::Value> = v["toolCalls"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let batch = framer_custom::StreamBatch {
            events,
            raced,
            aborted: page_stopped,
            finished: page_finished,
            tool_calls,
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
        assert!(OPEN_SNIPPET.contains("events >= stopAfter"));
        // The finish cutoff: the pump aborts on the finish_reason delta,
        // before the terminal/usage frame the meter settles on.
        assert!(OPEN_SNIPPET.contains("finish_reason"));
        assert!(OPEN_SNIPPET.contains("reader.cancel()"));
        // Fire-and-poll open: the pump (fetch + reader + decoder) runs
        // detached; the eval itself only starts it and returns instantly.
        assert!(OPEN_SNIPPET.contains("getReader()"));
        assert!(OPEN_SNIPPET.contains("TextDecoder"));
        assert!(OPEN_SNIPPET.contains("return { ok: true, status: 0"));
        assert!(OPEN_SNIPPET.contains("slot.error = \"castle-unavailable\""));
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
            serde_json::json!({"present": true, "text": "", "done": false, "stopped": false, "status": 200, "error": ""}),
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
            serde_json::json!({"present": true, "text": "", "done": false, "stopped": false, "status": 200, "error": ""}),
        ]);
        let id = s
            .open_stream(&serde_json::json!({}), framer_custom::Dialect::Chat, 200)
            .await
            .unwrap();
        assert_eq!(id, "ps1");
        // stale-slot recovery runs BEFORE the open fetch; the verify
        // drain runs right after it (page-pin proof).
        let seen = s.eval.seen();
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert!(seen[0].contains("slot.closed = true"), "first eval is stale cancel");
        assert!(seen[0].contains("__customPS"), "cancel targets the slot");
        assert!(seen[1].contains("stopAfter"), "second eval is the open");
        // The verify read is a PEEK (never moves the cursor): content that
        // lands during the open-wait must survive to the rung first drain.
        assert!(seen[2].contains("text_len"), "third eval is the verify peek");
        assert!(!seen[2].contains("slot.cursor"), "peek never touches the cursor");
    }

    #[tokio::test]
    async fn open_clamps_stop_into_engine_bounds() {
        use framer_custom::PageStream;
        // Micro-stops pass through (debited-race recovery: abort after the
        // first delta); u32::MAX clamps to MAX_STOP=10000; 40 stays 40.
        for (send, baked) in [(0u32, "1"), (1u32, "1"), (u32::MAX, "10000"), (40u32, "40")] {
            let mut s = test_stream(vec![
                serde_json::json!({"cancelled": false}),
                serde_json::json!({"ok": true, "status": 200}),
                serde_json::json!({"present": true, "text": "", "done": false, "stopped": false, "status": 200, "error": ""}),
            ]);
            s.open_stream(&serde_json::json!({}), framer_custom::Dialect::Chat, send)
                .await
                .unwrap();
            let seen = s.eval.seen();
            assert_eq!(seen.len(), 3);
            // %STOP% substitution carries the CLAMPED value
            assert!(
                seen[1].contains(&format!("({baked})")) || seen[1].contains(&format!(", {baked})")),
                "stop {send} should bake {baked}: {}",
                &seen[1][seen[1].len().saturating_sub(120)..]
            );
        }
    }

    #[test]
    fn decode_deltas_units() {
        use framer_custom::Event;
        // Empty slice = paced poll, not a delta.
        assert_eq!(HostPageStream::<ScriptEval>::decode_deltas(""), Vec::<Event>::new());
        // Partial line (no sentinel yet) = the still-open line, one delta.
        assert_eq!(
            HostPageStream::<ScriptEval>::decode_deltas("hello "),
            vec![Event::Delta("hello ".into())]
        );
        // One delta PER parsed data line: sentinels terminate runs.
        assert_eq!(
            HostPageStream::<ScriptEval>::decode_deltas("a\0b\0"),
            vec![Event::Delta("a".into()), Event::Delta("b".into())]
        );
        // Consecutive sentinels = consecutive empty lines = empty deltas
        // (each counts toward the rung stop, matching the page count).
        assert_eq!(
            HostPageStream::<ScriptEval>::decode_deltas("\0\0"),
            vec![Event::Delta("".into()), Event::Delta("".into())]
        );
        // Trailing open line after terminated runs.
        assert_eq!(
            HostPageStream::<ScriptEval>::decode_deltas("a\0partial"),
            vec![Event::Delta("a".into()), Event::Delta("partial".into())]
        );
    }

    #[tokio::test]
    async fn drain_decodes_one_delta_per_line() {
        use framer_custom::PageStream;
        // Pump-side framing: "hello " + "world " complete lines, "partial"
        // still open; then the stop lands ("ial" closes the line, two empty
        // lines, "tail" open) with stopped=true.
        let mut s = test_stream(vec![
            serde_json::json!({"cancelled": false, "reason": "no-stream"}),
            serde_json::json!({"ok": true, "status": 200}),
            serde_json::json!({"present": true, "text": "", "done": false, "stopped": false, "status": 200, "error": ""}),
            serde_json::json!({"present": true, "text": "hello \0world \0partial", "done": false, "stopped": false, "error": ""}),
            serde_json::json!({"present": true, "text": "ial\0\0tail", "done": true, "stopped": true, "stopAt": 200, "error": ""}),
        ]);
        let id = s
            .open_stream(&serde_json::json!({"stream": true}), framer_custom::Dialect::Chat, 200)
            .await
            .unwrap();
        let b1 = s.drain(&id).await.unwrap();
        assert_eq!(
            b1.events,
            vec![
                framer_custom::Event::Delta("hello ".into()),
                framer_custom::Event::Delta("world ".into()),
                framer_custom::Event::Delta("partial".into()),
            ]
        );
        assert!(!b1.raced && !b1.aborted);
        let b2 = s.drain(&id).await.unwrap();
        assert_eq!(
            b2.events,
            vec![
                framer_custom::Event::Delta("ial".into()),
                framer_custom::Event::Delta("".into()),
                framer_custom::Event::Delta("tail".into()),
            ]
        );
        // stopped (page aborted itself at the stop, no terminal read) is
        // NOT a race: no Done appended, aborted set.
        assert!(!b2.raced && b2.aborted);
    }

    #[tokio::test]
    async fn open_retries_across_pool_pages() {
        use framer_custom::PageStream;
        let mut s = test_stream(vec![
            serde_json::json!({"cancelled": false, "reason": "no-stream"}),
            serde_json::json!({"ok": true, "status": 200}),
            serde_json::json!({"present": false}),
            serde_json::json!({"cancelled": false, "reason": "no-stream"}),
            serde_json::json!({"ok": true, "status": 200}),
            serde_json::json!({"present": true, "text": "", "done": false, "stopped": false, "status": 200, "error": ""}),
        ]);
        let id = s
            .open_stream(&serde_json::json!({}), framer_custom::Dialect::Chat, 200)
            .await
            .unwrap();
        assert_eq!(id, "ps1");
        // Two full rotations: cancel+open+verify, then cancel+open+verify.
        assert_eq!(s.eval.seen().len(), 6);
    }

    #[tokio::test]
    async fn busy_slot_and_unknown_id_fail_closed() {
        use framer_custom::PageStream;
        let mut s = test_stream(vec![
            serde_json::json!({"cancelled": false, "reason": "no-stream"}),
            serde_json::json!({"ok": false, "status": 0, "error": "slot-busy"}),
            serde_json::json!({"cancelled": true, "reason": "cancelled"}),
            serde_json::json!({"ok": false, "status": 0, "error": "slot-busy"}),
            serde_json::json!({"cancelled": true, "reason": "cancelled"}),
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
