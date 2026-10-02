#!/usr/bin/env python3
"""diag-camoufox.py — does the Castle mint itself work inside Camoufox?
The page-global `castlePublishableAPIKey` never appears in Firefox (Framer's
config path differs), so the sidecar's gate was wrong. What actually matters:
can we import the wrapper chunk and mint a v3-shaped token?
Run:  ~/.framerize/venv/bin/python scripts/linux/diag-camoufox.py
"""
from camoufox.sync_api import Camoufox

CHUNK = "https://app.framerstatic.com/chunk-5LKSDTQ5.mjs"

MINT = """(async () => {
  const out = { keyBefore: typeof window.castlePublishableAPIKey, tries: [] };
  // THE FIX: Framer only serves the castle config inline to some UAs (never
  // Firefox). Publishable keys are public — inject when missing.
  if (!window.castlePublishableAPIKey) {
    Object.defineProperty(window, "castlePublishableAPIKey", { value: Object.freeze("pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB") });
  }
  out.keyAfter = typeof window.castlePublishableAPIKey;
  for (let i = 0; i < 4; i++) {
    try {
      const m = await import("%s?cb=" + Date.now() + "_" + i);
      const t = await m.a();
      out.tries.push(typeof t + " len=" + (t ? t.length : 0) + " pipeAt=" + (t ? t.indexOf("|") : -1));
      if (typeof t === "string" && t.length > 4000) break;
    } catch (e) {
      out.tries.push("ERR " + String(e).slice(0, 150));
    }
    await new Promise(r => setTimeout(r, 1500));
  }
  return out;
})()""" % CHUNK

with Camoufox(headless=True) as b:
    p = b.new_page()
    p.on("pageerror", lambda e: print("PAGEERROR:", str(e)[:200], flush=True))
    p.goto("https://framer.com/login", wait_until="domcontentloaded", timeout=90000)
    p.wait_for_timeout(5000)
    print("page state:", p.evaluate("({scripts: document.scripts.length, hasForm: !!document.querySelector('input[type=email], input[name=email]')})"), flush=True)
    print("minting...", flush=True)
    r = p.evaluate(MINT)
    print("RESULT:", r, flush=True)
    good = any("len=5" in t or "len=4" in t for t in r["tries"])
    ok = any("len=" in t and not t.startswith("ERR") and not t.startswith("undefined") for t in r["tries"])
    print("VERDICT:", "MINT WORKS IN CAMOUFOX (v3 or v2 — both backend-accepted)" if ok else "mint fails in camoufox — report this output")
