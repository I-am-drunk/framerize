/*
 * Framer AI Console — run AI inference through your logged-in Framer session.
 *
 * HOW TO USE
 * 1. In your own browser, log in to https://framer.com (this is "adding an account" —
 *    the script uses whatever account is signed in; switch accounts by signing
 *    out/in on framer.com and re-running).
 * 2. Open any project (URL looks like framer.com/projects/<Name>--<PROJECT_ID>).
 * 3. Open DevTools (F12) → Console, paste this whole file, press Enter.
 *    A small panel appears top-right. Type a prompt, hit Send.
 *
 * HOW IT WORKS (internal endpoints, captured from the Framer web app)
 * - GET  https://api.framer.com/auth/ai/access-token   (session cookie → 10-min Bearer JWT)
 * - POST https://api.framer.com/ai/v3/openai/responses/  (the Agent chat endpoint)
 * - Each request needs a fresh Castle anti-abuse token, generated exactly the way
 *   the app does it: import("…/chunk-5LKSDTQ5.mjs").a()  (wraps Castle v3 SDK).
 * - Remaining AI credits come back in the X-Framer-AI-Usage response header.
 *
 * NOTE: these are Framer-internal, undocumented endpoints and can change or break.
 */
(async () => {
  if (!location.hostname.endsWith("framer.com")) {
    alert("Run this on a framer.com page while logged in.");
    return;
  }
  if (window.__framerAIConsole) { window.__framerAIConsole.focus(); return; }

  const API = "https://api.framer.com";
  const CASTLE_MODULE = "https://app.framerstatic.com/chunk-5LKSDTQ5.mjs";

  const state = { token: null, tokenExpires: 0, sessionId: rid(), busy: false };
  function rid() { return Math.random().toString(36).slice(2, 11); }

  function projectIdFromUrl() {
    // Exactly 20 base62 chars; a trailing "-xxxx" is a project ACCESS TOKEN
    // (per bootstrap.js's regex), not part of the id.
    const m = location.pathname.match(/\/projects\/[^/]*--([A-Za-z0-9]{20})/);
    return m ? m[1] : null;
  }

  async function getAccessToken() {
    if (state.token && Date.now() < state.tokenExpires - 30000) return state.token;
    const r = await fetch(`${API}/auth/ai/access-token`, { credentials: "include" });
    if (r.status === 401 || r.status === 403) throw new Error("Not logged in — sign in at framer.com/login and re-run.");
    if (!r.ok) throw new Error(`access-token failed: HTTP ${r.status}`);
    const j = await r.json();
    state.token = j.accessToken;
    state.tokenExpires = new Date(j.expiresAt).getTime();
    return state.token;
  }

  function decodeJwt(jwt) {
    try { return JSON.parse(atob(jwt.split(".")[1].replace(/-/g, "+").replace(/_/g, "/"))); }
    catch { return null; }
  }

  async function getCastleToken() {
    const mod = await import(CASTLE_MODULE);
    return await mod.a(); // fresh Castle request token, same call the app makes
  }

  async function ask(prompt, onEvent) {
    const [accessToken, castleToken] = await Promise.all([getAccessToken(), getCastleToken()]);
    const projectId = projectIdFromUrl();
    const body = {
      projectId,
      featureCategory: "agents",
      featureName: "agents",
      step: "message",
      sessionId: state.sessionId,
      stepIndex: 0,
      turnId: rid(),
      userPrompt: prompt,
      castleToken,
      // REQUIRED — server 400s "request is required" without it. Model ids have
      // no provider prefix here ("gpt-6-sol", not "openai/gpt-6-sol").
      request: { model: "gpt-6-sol", input: prompt, stream: true },
    };
    const res = await fetch(`${API}/ai/v3/openai/responses/`, {
      method: "POST",
      credentials: "include",
      headers: { authorization: `Bearer ${accessToken}`, "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    const usage = res.headers.get("X-Framer-AI-Usage");
    if (!res.ok) {
      const txt = await res.text().catch(() => "");
      throw new Error(`HTTP ${res.status} ${txt.slice(0, 300)}`);
    }
    onEvent({ type: "usage", raw: usage });

    const ct = res.headers.get("content-type") || "";
    if (ct.includes("text/event-stream")) {
      const reader = res.body.getReader();
      const dec = new TextDecoder();
      let buf = "";
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        buf += dec.decode(value, { stream: true });
        let idx;
        while ((idx = buf.indexOf("\n\n")) >= 0) {
          const chunk = buf.slice(0, idx); buf = buf.slice(idx + 2);
          for (const line of chunk.split("\n")) {
            if (!line.startsWith("data:")) continue;
            const data = line.slice(5).trim();
            if (data === "[DONE]") continue;
            try {
              const ev = JSON.parse(data);
              const delta =
                ev.type === "response.output_text.delta" ? ev.delta :
                ev.type === "response.refusal.delta" ? ev.delta : null;
              if (delta) onEvent({ type: "delta", text: delta });
              else if (ev.type === "response.completed") onEvent({ type: "done", raw: ev, usage: ev.response?.usage });
              else if (ev.type === "error" || ev.type === "response.failed") onEvent({ type: "error", raw: ev });
            } catch { /* keep partial lines out */ }
          }
        }
      }
      onEvent({ type: "end" });
    } else {
      const txt = await res.text();
      onEvent({ type: "json", text: txt });
    }
  }

  // ---------- minimal panel UI ----------
  const el = document.createElement("div");
  el.id = "framer-ai-console";
  el.style.cssText = "position:fixed;top:12px;right:12px;z-index:999999;width:360px;background:#111;color:#eee;border:1px solid #333;border-radius:10px;font:13px/1.4 -apple-system,sans-serif;padding:10px;box-shadow:0 8px 30px rgba(0,0,0,.5)";
  el.innerHTML = `
    <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:6px">
      <b>Framer AI Console</b>
      <span data-x style="cursor:pointer;opacity:.6">✕</span>
    </div>
    <div data-acct style="opacity:.7;margin-bottom:6px">account: …</div>
    <div data-log style="height:220px;overflow:auto;background:#000;border-radius:6px;padding:6px;white-space:pre-wrap;word-break:break-word"></div>
    <textarea data-in rows="2" placeholder="Prompt…" style="width:100%;margin-top:6px;background:#000;color:#eee;border:1px solid #333;border-radius:6px;padding:6px;resize:vertical"></textarea>
    <div style="display:flex;justify-content:space-between;align-items:center;margin-top:6px">
      <span data-credits style="opacity:.7"></span>
      <button data-send style="background:#0099ff;color:#fff;border:0;border-radius:6px;padding:5px 14px;cursor:pointer">Send</button>
    </div>`;
  document.body.appendChild(el);
  const $ = (s) => el.querySelector(s);
  const log = (t) => { $("[data-log]").textContent += t; el.querySelector("[data-log]").scrollTop = 1e9; };

  $("[data-x]").onclick = () => { el.remove(); delete window.__framerAIConsole; };
  window.__framerAIConsole = { focus: () => $("[data-in]").focus(), ask };

  getAccessToken()
    .then((t) => {
      const p = decodeJwt(t);
      $("[data-acct]").textContent = "account: " + (p?.payload?.email || p?.payload?.username || "signed in");
    })
    .catch((e) => { $("[data-acct]").textContent = e.message; });

  async function send() {
    if (state.busy) return;
    const prompt = $("[data-in]").value.trim();
    if (!prompt) return;
    $("[data-in]").value = "";
    state.busy = true;
    log(`\nYou: ${prompt}\nAI: `);
    try {
      await ask(prompt, (ev) => {
        if (ev.type === "delta") log(ev.text);
        else if (ev.type === "json") log(ev.text + "\n");
        else if (ev.type === "usage" && ev.raw) {
          const rem = new URLSearchParams(ev.raw).get("remaining");
          if (rem) $("[data-credits]").textContent = `credits left: ${rem}`;
        } else if (ev.type === "error") log(`\n[stream error] ${JSON.stringify(ev.raw).slice(0, 300)}\n`);
      });
      log("\n");
    } catch (e) {
      log(`\n[error] ${e.message}\n`);
    } finally {
      state.busy = false;
    }
  }
  $("[data-send]").onclick = send;
  $("[data-in]").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); }
  });
  $("[data-in]").focus();
})();
