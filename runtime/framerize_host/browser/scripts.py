"""Browser-page operations used by the host."""
import json
from pathlib import Path
from ..settings import API, CASTLE_CHUNK
from .responses import BODY_LIMIT

AI_RESPONSE_LIMIT = BODY_LIMIT
_READ_RESPONSE_JS = (
    '(response, controller) => ('
    + Path(__file__).with_name('response_reader.js').read_text()
    + f')(response, controller, job, {AI_RESPONSE_LIMIT})')

WARM_JS = """(async () => {
  if (!window.castlePublishableAPIKey) {
    Object.defineProperty(window, "castlePublishableAPIKey", { value: Object.freeze("pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB") });
  }
  try {
    if (!window.__fzCastleM) {
      const url = "%s";
      const module = await import(url);
      window.__fzCastleM = module;
      window.__fzResidentWrapper = Object.freeze({
        module, callable: module.a, url, entry: "a", realm: window
      });
    }
    const t = await window.__fzCastleM.a();
    if (typeof t !== "string" || t.length <= 500)
      throw new Error("Castle SDK returned no usable token during warmup");
    return true;
  } catch (e) { throw e; }
})()""" % CASTLE_CHUNK


MINT_JS = """(async () => {
  if (!window.castlePublishableAPIKey) {
    Object.defineProperty(window, "castlePublishableAPIKey", { value: Object.freeze("pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB") });
  }
  // Import the wrapper ONCE per page and cache it. Cache-busting per retry
  // was a 5s-per-attempt mistake: each fresh module pays the wrapper's 5s v3
  // race again (v3 always loses it in Firefox, then latches off — but the
  // latch only persists within one module instance). With a cached module,
  // the first mint pays the race once; every later mint is ~1s (v2 path).
  if (!window.__fzCastleM) {
    const url = "%s";
    const module = await import(url);
    window.__fzCastleM = module;
    window.__fzResidentWrapper = Object.freeze({
      module, callable: module.a, url, entry: "a", realm: window
    });
  }
  const m = window.__fzCastleM;
  let failure;
  for (let i = 0; i < 3; i++) {
    try {
      const t = await m.a();
      if (typeof t === "string" && t.length > 500) return t;
    } catch (e) { failure = e; }
    await new Promise(r => setTimeout(r, 1500));
  }
  // Module instances and their listeners live until the page closes.
  // Keep the installed wrapper and report its failure without re-importing.
  if (failure) throw failure;
  throw new Error("Castle SDK did not return a usable token after three calls");
})()""" % CASTLE_CHUNK


def post_js(path, body, bearer=None):
    """Mint inline, then POST. Used for AUTH calls (signin/continue), which are
    one-at-a-time by nature. AI calls use post_with_token_js instead so the mint
    can be gated per account -- see MINT_GAP_S."""
    auth = '"authorization": "Bearer %s",' % bearer if bearer else ""
    return """(async () => {
  const tok = await (%s);
  if (!tok) return { status: 0, usage: null, body: "browser token acquisition failed: Castle SDK returned no usable token" };
  const body = %s;
  body.castleToken = tok;
  try {
    const r = await fetch("%s" + "%s", {
      method: "POST", credentials: "include",
      headers: { "content-type": "application/json; charset=utf-8", "x-requested-by": "Framer", %s },
      body: JSON.stringify(body),
    });
    return { status: r.status, usage: r.headers.get("x-framer-ai-usage"), body: await r.text() };
  } catch (e) { return { status: -1, usage: null, body: String(e) }; }
})()""" % (MINT_JS, json.dumps(body), API, path, auth)


AI_START_JS = """(async ([path, body, token, bearer, jobId, dev]) => {
  const readResponse = %s;
  // The velocity/concurrency counters key on the page's __cuid (measured
  // 2026-10-05: spliced tokens claiming other devices do NOT multiply the
  // bucket). Align the ambient device with the token's claimed device BEFORE
  // firing — pooled tokens minted for device Y must not be counted against
  // whatever device the last mint left behind.
  if (dev) {
    try { localStorage.setItem('__cuid', dev); } catch(e) {}
    try { document.cookie = '__cuid=' + dev + '; domain=.framer.com; path=/; secure; samesite=None'; } catch(e) {}
  }
  // FIRE-AND-POLL (2026-10-04): the browser driver serializes long-lived
  // protocol calls per process — an evaluate that awaits a 100s fetch holds
  // EVERY other command behind it (this was the 46s-mint / phantom-collapse
  // mechanism). So the fetch starts in-page and this call returns instantly;
  // completion is polled with instant evaluates (AI_POLL_JS).
  window.__aiJobs = window.__aiJobs || {};
  const headers = { "content-type": "application/json; charset=utf-8", "x-requested-by": "Framer" };
  if (bearer) headers["authorization"] = "Bearer " + bearer;
  body.castleToken = token;
  const job = { done: false, value: null, controller: new AbortController() };
  window.__aiJobs[jobId] = job;
  // A disconnected host must not leave a fetch or its response resident.
  job.timer = setTimeout(() => {
    if (window.__aiJobs[jobId] !== job) return;
    job.controller.abort();
    // Keep the job joinable until its body reader has actually settled.
    job.settled.then(() => {
      if (window.__aiJobs[jobId] === job) delete window.__aiJobs[jobId];
    });
  }, 900000);
  job.settled = (async () => {
    let failurePhase = "fetch", headStatus = null;
    try {
      const r = await fetch("%s" + path, {
        method: "POST", credentials: "include", headers, body: JSON.stringify(body),
        signal: job.controller.signal,
      });
      headStatus = r.status;
      failurePhase = "body";
      return { status: r.status, usage: r.headers.get("x-framer-ai-usage"),
        body: await readResponse(r, job.controller) };
    } catch (e) {
      return { status: -1, usage: null, body: String(e),
        failure_phase: failurePhase, head_status: headStatus };
    }
  })().then(v => {
    // Cancellation deletes the slot. A late completion must not recreate it.
    if (window.__aiJobs[jobId] === job) {
      job.done = true;
      job.value = v;
    }
  });
  return jobId;
})""" % (_READ_RESPONSE_JS, API)


AI_POLL_JS = Path(__file__).with_name('response_poll.js').read_text()


AI_CANCEL_JS = """(async (jobId) => {
  const jobs = window.__aiJobs || {};
  const job = jobs[jobId];
  if (!job) return false;
  delete jobs[jobId];
  clearTimeout(job.timer);
  job.controller.abort();
  // The host retains this RPC's account owner until actual fetch settlement.
  await job.settled;
  return true;
})"""


DEVPOOL_WARM_JS = """(async () => {
  // Build the device pool: K castle-module instances, each bound to its own
  // __cuid. Modules are singletons per URL, so each device needs its own
  // import URL — warmed CONCURRENTLY (each pays the v3 race in parallel).
  if (!window.__devmods) window.__devmods = {};
  if (!window.castlePublishableAPIKey) Object.defineProperty(window, 'castlePublishableAPIKey', { value: Object.freeze('pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB') });
  const ids = %s; // JSON array of uuids, persisted per account
  const saved = (() => { try { return localStorage.getItem('__cuid'); } catch(e) { return null; } })();
  // SEQUENTIAL, never Promise.all: a module binds the device it sees at
  // import time, and parallel swaps race — every module ends up on the LAST
  // __cuid and the pool silently collapses to one device (measured: 429s).
  for (let i = 0; i < ids.length; i++) {
    const dev = ids[i];
    if (window.__devmods[dev]) continue;
    try {
      localStorage.setItem('__cuid', dev);
      document.cookie = '__cuid=' + dev + '; domain=.framer.com; path=/; secure; samesite=None';
      window.__devmods[dev] = await import('https://app.framerstatic.com/chunk-5LKSDTQ5.mjs?dev=' + i);
    } catch(e) {}
  }
  if (saved) {
    try { localStorage.setItem('__cuid', saved); } catch(e) {}
    document.cookie = '__cuid=' + saved + '; domain=.framer.com; path=/; secure; samesite=None';
  }
  return Object.keys(window.__devmods).length;
})()"""


DEVPOOL_MINT_JS = """(async ([ids, wantDev]) => {
  if (!window.__devmods || Object.keys(window.__devmods).length === 0) return null;
  window.__devidx = ((window.__devidx || 0) + 1) % ids.length;
  const dev = (wantDev && window.__devmods[wantDev]) ? wantDev : ids[window.__devidx];
  const mod = window.__devmods[dev];
  if (!mod) return null;
  // Keep the ambient device aligned with the minting module (the AI request
  // fires right after this mint, from this page).
  try {
    localStorage.setItem('__cuid', dev);
    document.cookie = '__cuid=' + dev + '; domain=.framer.com; path=/; secure; samesite=None';
  } catch(e) {}
  for (let i = 0; i < 3; i++) {
    const t = await mod.a();
    if (t) return { t, dev };
    await new Promise(r => setTimeout(r, 600));
  }
  return null;
})"""


SAFE_CLICK_JS = """([x, y]) => {
  const el = document.elementFromPoint(x, y);
  if (!el) return false;
  if (el === document.body || el === document.documentElement) return true;
  if (el.closest('a,button,input,select,textarea,[role="button"],form')) return false;
  return el.tagName === 'DIV' || el.tagName === 'MAIN' || el.tagName === 'SECTION';
}"""
