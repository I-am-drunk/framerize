#!/usr/bin/env python3
"""diag-camoufox.py — v4: dissect the Castle V3 failure inside Camoufox.
v2 minting works (Framer's own fallback). v3 doesn't. Drive the v3 SDK
manually — import, configure, createRequestToken — with NO timeouts and
capture the exact failure point.
Run:  ~/.framerize/venv/bin/python scripts/linux/diag-camoufox.py
"""
from camoufox.sync_api import Camoufox

PK = "pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB"
SDK = "https://app.framerstatic.com/castleSDKV3-LNGA36I3.mjs"

V3_PROBE = """(async () => {
  const out = { phase: "start" };
  if (!window.castlePublishableAPIKey) {
    Object.defineProperty(window, "castlePublishableAPIKey", { value: Object.freeze("%s") });
  }
  try {
    out.phase = "module-load";
    const m = await import("%s?cb=" + Date.now());
    out.moduleLoaded = true;
    out.exports = Object.keys(m).slice(0, 12);
    out.phase = "configure";
    const inst = await m.configure({ pk: window.castlePublishableAPIKey, onError: (e) => { out.sdkOnError = String(e && e.message || e).slice(0, 300); } });
    out.configured = !!inst;
    out.instMethods = inst ? Object.keys(inst).slice(0, 12) : [];
    out.phase = "token-generation";
    const t0 = performance.now();
    const tok = await inst.createRequestToken();
    out.genMs = Math.round(performance.now() - t0);
    out.token = typeof tok + " len=" + (tok ? tok.length : 0) + " pipeAt=" + (tok ? tok.indexOf("|") : -1) + " head=" + (tok ? tok.slice(0, 12) : "");
    out.phase = "done";
  } catch (e) {
    out.error = String(e && e.message || e).slice(0, 300);
    out.errorName = e && e.name;
    out.stackHead = String(e && e.stack || "").slice(0, 400);
  }
  return out;
})()""" % (PK, SDK)

with Camoufox(headless=True) as b:
    p = b.new_page()
    p.on("pageerror", lambda e: print("PAGEERROR:", str(e)[:250], flush=True))
    p.goto("https://framer.com/login", wait_until="domcontentloaded", timeout=90000)
    p.wait_for_timeout(4000)
    print("probing v3 SDK directly (no timeouts)...", flush=True)
    r = p.evaluate(V3_PROBE)
    for k, v in r.items():
        print(f"  {k}: {v}", flush=True)
    phase = r.get("phase")
    if phase == "done":
        print("VERDICT: v3 works when driven manually — the wrapper's 5s race is the problem")
    else:
        print(f"VERDICT: v3 fails at phase '{phase}' — see error fields above")
