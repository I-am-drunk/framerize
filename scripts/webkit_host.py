#!/usr/bin/env python3
"""webkit_host.py — framerize WebKit sidecar.

One shared WebKit browser; each account = one isolated browser context with
persistent storage state (cookies + localStorage, incl. the httpOnly session
cookie and Castle's __cuid — no manual seeding needed). Measured: 126MB for
the first account, ~0 marginal MB per extra context (2026-10-01 benchmark).

Protocol: newline-delimited JSON on stdin/stdout.
  -> {"id": 1, "cmd": "begin",    "account": "<uuid>", "email": "..."}
  -> {"id": 2, "cmd": "complete", "account": "<uuid>", "token": "<uuid>"}
  -> {"id": 3, "cmd": "eval",     "account": "<uuid>", "expr": "<js>"}
  -> {"id": 4, "cmd": "close",    "account": "<uuid>"}
  <- {"id": 1, "ok": true, "result": ...}  /  {"id": 1, "ok": false, "error": "..."}

State root: $FRAMERIZE_STATE (default ~/.framerize/webkit); per-account
storage_state at <root>/<account>.json. Castle v3 is enforced by shape
(>4000 chars, '|' at index 8); the v2 fallback is never returned.
"""
import json, os, sys, threading, time

from playwright.sync_api import sync_playwright

def trace(msg):
    sys.stderr.write(f"[{time.strftime('%H:%M:%S')}] {msg}\n")
    sys.stderr.flush()

ROOT = os.environ.get("FRAMERIZE_STATE", os.path.expanduser("~/.framerize/webkit"))
LOGIN_URL = "https://framer.com/login"
API = "https://api.framer.com"
CASTLE_CHUNK = "https://app.framerstatic.com/chunk-5LKSDTQ5.mjs"

# Warm-mint loop: cache-busted imports (fresh module => un-latched Rh flag),
# accept only v3-shaped tokens. Returns null when v3 stays unavailable.
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
    window.__fzCastleM = await import("%s");
  }
  const m = window.__fzCastleM;
  for (let i = 0; i < 3; i++) {
    try {
      const t = await m.a();
      if (typeof t === "string" && t.length > 500) return t;
    } catch (e) {}
    await new Promise(r => setTimeout(r, 1500));
  }
  // Recovery path only: one fresh module (fresh Rh latch) for transient
  // cold-load races (the original reason for cache-busting).
  try {
    const fresh = await import("%s?cb=" + Date.now());
    const t = await fresh.a();
    if (typeof t === "string" && t.length > 500) { window.__fzCastleM = fresh; return t; }
  } catch (e) {}
  return null;
})()""" % (CASTLE_CHUNK, CASTLE_CHUNK)


def auth_post_js(path, body):
    return """(async () => {
  const tok = await (%s);
  if (!tok) return { status: 0, body: "castle-v3-unavailable" };
  const body = %s;
  body.castleToken = tok;
  try {
    const r = await fetch("%s" + "%s", {
      method: "POST", credentials: "include",
      headers: { "content-type": "application/json; charset=utf-8", "x-requested-by": "Framer" },
      body: JSON.stringify(body),
    });
    return { status: r.status, body: await r.text() };
  } catch (e) { return { status: -1, body: String(e) }; }
})()""" % (MINT_JS, json.dumps(body), API, path)


class Host:
    def __init__(self):
        os.makedirs(ROOT, exist_ok=True)
        self.pw = sync_playwright().start()
        self.browser = self.pw.webkit.launch()
        self.accounts = {}  # account -> (context, page)
        self.lock = threading.Lock()

    def _page(self, account, proxy=None):
        if account in self.accounts:
            return self.accounts[account][1]
        state_path = os.path.join(ROOT, f"{account}.json")
        kwargs = {}
        if os.path.exists(state_path):
            kwargs["storage_state"] = state_path
        # Per-account egress: Castle binds tokens to egress IP (verified live:
        # clean token + flagged IP = 403 on /ai/*). Accounts are networks.
        if proxy:
            kwargs["proxy"] = {"server": proxy}
        ctx = self.browser.new_context(**kwargs)
        page = ctx.new_page()
        trace(f"{account[:8]}: goto login page")
        page.goto(LOGIN_URL, wait_until="domcontentloaded", timeout=60000)
        trace(f"{account[:8]}: dom loaded, waiting for castle key")
        page.wait_for_function("typeof window.castlePublishableAPIKey !== 'undefined'", timeout=30000)
        trace(f"{account[:8]}: page ready")
        self.accounts[account] = (ctx, page)
        return page

    def _save(self, account):
        if account in self.accounts:
            self.accounts[account][0].storage_state(path=os.path.join(ROOT, f"{account}.json"))

    def handle(self, req):
        cmd = req.get("cmd")
        account = req.get("account", "")
        if cmd == "begin":
            page = self._page(account, req.get("proxy"))
            trace(f"{account[:8]}: mint+signin eval start")
            r = page.evaluate(auth_post_js("/auth/v4/signin", {"email": req["email"], "origin": "web"}))
            trace(f"{account[:8]}: signin -> status {r['status']}")
            if not (200 <= r["status"] < 300):
                raise RuntimeError(f"signin {r['status']}: {r['body'][:400]}")
            self._save(account)
            return {"status": r["status"]}
        if cmd == "complete":
            page = self._page(account)
            r = page.evaluate(auth_post_js("/auth/v4/continue", {"token": req["token"]}))
            if not (200 <= r["status"] < 300):
                raise RuntimeError(f"continue {r['status']}: {r['body'][:400]}")
            self._save(account)  # persists the session cookie
            me = page.evaluate("""(async () => {
              const r = await fetch("%s/web/users/me", { headers: { "x-requested-by": "Framer" }, credentials: "include" });
              return { status: r.status, body: await r.text() };
            })()""" % API)
            if not (200 <= me["status"] < 300):
                raise RuntimeError(f"me {me['status']}: {me['body'][:400]}")
            cookies = self.accounts[account][0].cookies("https://framer.com")
            session = next((c["value"] for c in cookies if c["name"] == "session"), None)
            if not session:
                raise RuntimeError("no session cookie after continue")
            return {"cookie_header": f"session={session}", "me": json.loads(me["body"])}
        if cmd == "eval":
            page = self._page(account)
            return page.evaluate(req["expr"])
        if cmd == "close":
            if account in self.accounts:
                self._save(account)
                self.accounts.pop(account)[0].close()
            return True
        raise RuntimeError(f"unknown cmd {cmd}")

    def loop(self):
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            try:
                req = json.loads(line)
                with self.lock:
                    result = self.handle(req)
                out = {"id": req.get("id"), "ok": True, "result": result}
            except Exception as e:
                try:
                    rid = json.loads(line).get("id")
                except Exception:
                    rid = None
                out = {"id": rid, "ok": False, "error": str(e)[:800]}
            sys.stdout.write(json.dumps(out) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    Host().loop()
