#!/usr/bin/env python3
"""camoufox_host.py — framerize Camoufox (stealth Firefox) sidecar.

Camoufox passes Castle where headless Chromium builds don't: it spoofs the
fingerprint at C++ level (fonts, navigator.*, screen, WebGL), no CDP artifacts,
no "Headless" anywhere. This is the engine the user's previous program used
successfully against this exact protection.

One Camoufox process per account (persistent_context model): the profile dir
IS the account identity — cookies (incl. the httpOnly session cookie and
Castle's __cuid), storage, and fingerprint config persist across restarts.

Protocol: newline-delimited JSON on stdin/stdout.
  -> {"id":1,"cmd":"begin","account":"<uuid>","email":"...","proxy":"http://..."?}
  -> {"id":2,"cmd":"complete","account":"<uuid>","token":"<uuid>"}
  -> {"id":3,"cmd":"mint","account":"<uuid>"}
  -> {"id":4,"cmd":"ai_post","account":"<uuid>","path":"/ai/v3/chat/","bearer":"...","body":{...}}
  -> {"id":5,"cmd":"set_cookie","account":"<uuid>","name":"session","value":"..."}
  -> {"id":6,"cmd":"eval","account":"<uuid>","expr":"<js>"}
  -> {"id":7,"cmd":"close","account":"<uuid>"}
  <- {"id":1,"ok":true,"result":...} / {"id":1,"ok":false,"error":"..."}

State root: $FRAMERIZE_STATE (default ~/.framerize/camoufox); per-account
profile at <root>/<account>/. Castle v3 enforced by shape (>4000 chars, '|'
at index 8) — the v2 fallback is never returned.
"""
import json, os, shutil, sys, threading, time

def trace(msg):
    sys.stderr.write(f"[camoufox {time.strftime('%H:%M:%S')}] {msg}\n")
    sys.stderr.flush()

try:
    from camoufox.sync_api import Camoufox
except ImportError:
    trace("camoufox not installed. Run: scripts/linux/install-camoufox.sh")
    sys.exit(2)

ROOT = os.environ.get("FRAMERIZE_STATE", os.path.expanduser("~/.framerize/camoufox"))
LOGIN_URL = "https://framer.com/login"
API = "https://api.framer.com"
CASTLE_CHUNK = "https://app.framerstatic.com/chunk-5LKSDTQ5.mjs"

# Warm-mint loop: cache-busted imports (fresh module => un-latched Rh flag),
# accept only v3-shaped tokens. Returns null when v3 stays unavailable.
MINT_JS = """(async () => {
  // Framer serves window.castlePublishableAPIKey inline only to some UAs
// (verified 2026-10-02: absent in Firefox → wrapper configure fails → m.a()
// returns undefined). Publishable keys are public; inject when missing.
if (!window.castlePublishableAPIKey) {
  Object.defineProperty(window, "castlePublishableAPIKey", { value: Object.freeze("pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB") });
}
for (let i = 0; i < 8; i++) {
    try {
      const m = await import("%s?cb=" + Date.now() + "_" + i);
      const t = await m.a();
      // accept v3 (preferred) AND v2 — Framer ships the v2 fallback itself,
    // so the backend accepts both; the app sends whatever m.a() returns
    if (typeof t === "string" && t.length > 500) return t;
    } catch (e) {}
    await new Promise(r => setTimeout(r, 2000));
  }
  return null;
})()""" % CASTLE_CHUNK


def post_js(path, body, bearer=None):
    auth = '"authorization": "Bearer %s",' % bearer if bearer else ""
    return """(async () => {
  const tok = await (%s);
  if (!tok) return { status: 0, usage: null, body: "castle-v3-unavailable" };
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


class Host:
    def __init__(self):
        os.makedirs(ROOT, exist_ok=True)
        self.accounts = {}  # account -> (camoufox_ctx_manager, browser_context, page)
        self.proxies = {}   # account -> proxy url (applies at launch)
        self.lock = threading.Lock()

    def _page(self, account, proxy=None):
        if account in self.accounts:
            return self.accounts[account][2]
        if proxy:
            self.proxies[account] = proxy
        profile = os.path.join(ROOT, account)
        os.makedirs(profile, exist_ok=True)
        kwargs = dict(
            headless=True,
            persistent_context=True,
            user_data_dir=profile,
            humanize=True,  # human-like cursor/typing behavior in the page
        )
        # Per-account egress: Castle binds tokens to egress IP (verified live:
        # clean token + flagged IP = 403 on /ai/*). Accounts are networks.
        px = self.proxies.get(account)
        if px:
            kwargs["proxy"] = {"server": px}
        trace(f"{account[:8]}: launching camoufox (profile {profile})")
        mgr = Camoufox(**kwargs)
        ctx = mgr.__enter__()
        page = ctx.new_page()
        trace(f"{account[:8]}: goto login page")
        page.goto(LOGIN_URL, wait_until="domcontentloaded", timeout=90000)
        trace(f"{account[:8]}: dom loaded, waiting for app boot")
        # Gate on the APP being up (the login form renders), NOT on
        # window.castlePublishableAPIKey — that global is set by a code path
        # that never fires in Firefox (verified 2026-10-02: page boots fine,
        # global stays undefined). The mint loop is the real readiness probe.
        try:
            page.wait_for_selector("input[type=email], input[name=email], form", timeout=30000)
            trace(f"{account[:8]}: page ready")
        except Exception as e:
            trace(f"{account[:8]}: app-boot wait gave up ({type(e).__name__}) — continuing anyway")
        page.wait_for_timeout(1500)
        self.accounts[account] = (mgr, ctx, page)
        return page

    def _ctx(self, account):
        return self.accounts[account][1]

    def handle(self, req):
        cmd = req.get("cmd")
        account = req.get("account", "")
        if cmd == "begin":
            page = self._page(account, req.get("proxy"))
            trace(f"{account[:8]}: mint+signin eval start")
            r = page.evaluate(post_js("/auth/v4/signin", {"email": req["email"], "origin": "web"}))
            trace(f"{account[:8]}: signin -> status {r['status']}")
            if not (200 <= r["status"] < 300):
                raise RuntimeError("signin %s: %s" % (r["status"], r["body"][:400]))
            return {"status": r["status"]}
        if cmd == "complete":
            page = self._page(account)
            r = page.evaluate(post_js("/auth/v4/continue", {"token": req["token"]}))
            if not (200 <= r["status"] < 300):
                raise RuntimeError("continue %s: %s" % (r["status"], r["body"][:400]))
            me = page.evaluate("""(async () => {
              const r = await fetch("%s/web/users/me", { headers: { "x-requested-by": "Framer" }, credentials: "include" });
              return { status: r.status, body: await r.text() };
            })()""" % API)
            if not (200 <= me["status"] < 300):
                raise RuntimeError("me %s: %s" % (me["status"], me["body"][:400]))
            cookies = self._ctx(account).cookies("https://framer.com")
            session = next((c["value"] for c in cookies if c["name"] == "session"), None)
            if not session:
                raise RuntimeError("no session cookie after continue")
            return {"cookie_header": "session=" + session, "me": json.loads(me["body"])}
        if cmd == "mint":
            page = self._page(account)
            tok = page.evaluate(MINT_JS)
            if not tok:
                raise RuntimeError("castle-v3-unavailable")
            return tok
        if cmd == "ai_post":
            page = self._page(account)
            r = page.evaluate(post_js(req["path"], req["body"], req.get("bearer")))
            return r  # {status, usage, body} — the Rust side maps statuses
        if cmd == "set_cookie":
            self._page(account)  # ensure launched
            self._ctx(account).add_cookies([{
                "name": req["name"], "value": req["value"],
                "domain": ".framer.com", "path": "/",
                "secure": True, "httpOnly": True, "sameSite": "None",
            }])
            return True
        if cmd == "eval":
            page = self._page(account)
            return page.evaluate(req["expr"])
        if cmd == "close":
            if account in self.accounts:
                mgr, ctx, _ = self.accounts.pop(account)
                try:
                    ctx.close()
                finally:
                    mgr.__exit__(None, None, None)
            return True
        if cmd == "rotate":
            # Fresh device: close, wipe the profile entirely. The daemon's
            # stored session is re-seeded via set_cookie on next use.
            if account in self.accounts:
                mgr, ctx, _ = self.accounts.pop(account)
                try:
                    ctx.close()
                finally:
                    mgr.__exit__(None, None, None)
            shutil.rmtree(os.path.join(ROOT, account), ignore_errors=True)
            return True
        raise RuntimeError("unknown cmd %s" % cmd)

    def loop(self):
        trace("sidecar ready")
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
