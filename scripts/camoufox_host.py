#!/usr/bin/env python3
"""camoufox_host.py — framerize Camoufox (stealth Firefox) sidecar. ASYNC.

AsyncCamoufox + asyncio throughout: the sync API dies after the first launch
("Playwright Sync API inside the asyncio loop") because a Camoufox launch
leaves a running loop in the calling thread. Async inside the loop is the
prescribed model (learned 2026-10-02, second-login 500).

One Camoufox process per account (persistent_context model): the profile dir
IS the account identity — cookies (incl. the httpOnly session cookie and
Castle's __cuid), storage, and fingerprint config persist across restarts.

Protocol: newline-delimited JSON on stdin/stdout (unchanged).
  -> {"id":1,"cmd":"begin","account":"<uuid>","email":"...","proxy":"http://..."?}
  -> {"id":2,"cmd":"complete","account":"<uuid>","token":"<uuid>"}
  -> {"id":3,"cmd":"mint","account":"<uuid>"}
  -> {"id":4,"cmd":"ai_post","account":"<uuid>","path":"/ai/v3/chat/","bearer":"...","body":{...}}
  -> {"id":5,"cmd":"set_cookie","account":"<uuid>","name":"session","value":"..."}
  -> {"id":6,"cmd":"eval","account":"<uuid>","expr":"<js>"}
  -> {"id":7,"cmd":"close","account":"<uuid>"}
  -> {"id":8,"cmd":"rotate","account":"<uuid>"}
  <- {"id":1,"ok":true,"result":...} / {"id":1,"ok":false,"error":"..."}

State root: $FRAMERIZE_STATE (default ~/.framerize/camoufox). Tokens: accept
v3 AND v2 (Framer ships the v2 fallback itself; Firefox mints v2).
"""
import asyncio, json, os, shutil, sys, time

def trace(msg):
    sys.stderr.write(f"[camoufox {time.strftime('%H:%M:%S')}] {msg}\n")
    sys.stderr.flush()

try:
    from camoufox.async_api import AsyncCamoufox
except ImportError:
    trace("camoufox not installed. Run: scripts/linux/install-camoufox.sh")
    sys.exit(2)

ROOT = os.environ.get("FRAMERIZE_STATE", os.path.expanduser("~/.framerize/camoufox"))
LOGIN_URL = "https://framer.com/login"
API = "https://api.framer.com"
CASTLE_CHUNK = "https://app.framerstatic.com/chunk-5LKSDTQ5.mjs"

# Framer serves window.castlePublishableAPIKey inline only to some UAs (never
# Firefox — verified 2026-10-02). Publishable keys are public; inject it.
# Accept v3 (preferred) AND v2 (Framer's own fallback) shaped tokens.
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
})()""" % (CASTLE_CHUNK, CASTLE_CHUNK) % CASTLE_CHUNK


def post_js(path, body, bearer=None):
    auth = '"authorization": "Bearer %s",' % bearer if bearer else ""
    return """(async () => {
  const tok = await (%s);
  if (!tok) return { status: 0, usage: null, body: "castle-unavailable" };
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
        self.accounts = {}  # account -> (mgr, ctx, page)
        self.proxies = {}
        self.lock = asyncio.Lock()

    async def _page(self, account, proxy=None):
        if account in self.accounts:
            return self.accounts[account][2]
        if proxy:
            self.proxies[account] = proxy
        profile = os.path.join(ROOT, account)
        os.makedirs(profile, exist_ok=True)
        kwargs = dict(headless=True, persistent_context=True, user_data_dir=profile, humanize=True)
        # Per-account egress: Castle binds tokens to egress IP (verified live).
        px = self.proxies.get(account)
        if px:
            kwargs["proxy"] = {"server": px}
        trace(f"{account[:8]}: launching camoufox (profile {profile})")
        mgr = AsyncCamoufox(**kwargs)
        ctx = await mgr.__aenter__()
        page = await ctx.new_page()
        trace(f"{account[:8]}: goto login page")
        await page.goto(LOGIN_URL, wait_until="domcontentloaded", timeout=90000)
        trace(f"{account[:8]}: dom loaded, waiting for app boot")
        # Gate on the APP being up (login form renders), NOT on
        # window.castlePublishableAPIKey — Firefox never gets that global.
        try:
            await page.wait_for_selector("input[type=email], input[name=email], form", timeout=30000)
            trace(f"{account[:8]}: page ready")
        except Exception as e:
            trace(f"{account[:8]}: app-boot wait gave up ({type(e).__name__}) — continuing anyway")
        await page.wait_for_timeout(1500)
        self.accounts[account] = (mgr, ctx, page)
        return page

    def _ctx(self, account):
        return self.accounts[account][1]

    async def handle(self, req):
        cmd = req.get("cmd")
        account = req.get("account", "")
        if cmd == "begin":
            page = await self._page(account, req.get("proxy"))
            trace(f"{account[:8]}: mint+signin eval start")
            r = await page.evaluate(post_js("/auth/v4/signin", {"email": req["email"], "origin": "web"}))
            trace(f"{account[:8]}: signin -> status {r['status']}")
            if not (200 <= r["status"] < 300):
                raise RuntimeError("signin %s: %s" % (r["status"], r["body"][:400]))
            return {"status": r["status"]}
        if cmd == "complete":
            page = await self._page(account)
            r = await page.evaluate(post_js("/auth/v4/continue", {"token": req["token"]}))
            if not (200 <= r["status"] < 300):
                raise RuntimeError("continue %s: %s" % (r["status"], r["body"][:400]))
            me = await page.evaluate("""(async () => {
              const r = await fetch("%s/web/users/me", { headers: { "x-requested-by": "Framer" }, credentials: "include" });
              return { status: r.status, body: await r.text() };
            })()""" % API)
            if not (200 <= me["status"] < 300):
                raise RuntimeError("me %s: %s" % (me["status"], me["body"][:400]))
            cookies = await self._ctx(account).cookies("https://framer.com")
            session = next((c["value"] for c in cookies if c["name"] == "session"), None)
            if not session:
                raise RuntimeError("no session cookie after continue")
            return {"cookie_header": "session=" + session, "me": json.loads(me["body"])}
        if cmd == "mint":
            page = await self._page(account)
            tok = await page.evaluate(MINT_JS)
            if not tok:
                raise RuntimeError("castle-unavailable")
            return tok
        if cmd == "ai_post":
            page = await self._page(account)
            return await page.evaluate(post_js(req["path"], req["body"], req.get("bearer")))
        if cmd == "set_cookie":
            await self._page(account)
            await self._ctx(account).add_cookies([{
                "name": req["name"], "value": req["value"],
                "domain": ".framer.com", "path": "/",
                "secure": True, "httpOnly": True, "sameSite": "None",
            }])
            return True
        if cmd == "eval":
            page = await self._page(account)
            return await page.evaluate(req["expr"])
        if cmd == "close":
            if account in self.accounts:
                mgr, ctx, _ = self.accounts.pop(account)
                try:
                    await ctx.close()
                finally:
                    await mgr.__aexit__(None, None, None)
            return True
        if cmd == "rotate":
            if account in self.accounts:
                mgr, ctx, _ = self.accounts.pop(account)
                try:
                    await ctx.close()
                finally:
                    await mgr.__aexit__(None, None, None)
            shutil.rmtree(os.path.join(ROOT, account), ignore_errors=True)
            return True
        raise RuntimeError("unknown cmd %s" % cmd)

    async def loop(self):
        trace("sidecar ready (async)")
        loop = asyncio.get_running_loop()
        while True:
            line = await loop.run_in_executor(None, sys.stdin.readline)
            if not line:
                return
            line = line.strip()
            if not line:
                continue
            try:
                req = json.loads(line)
                async with self.lock:
                    result = await self.handle(req)
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
    asyncio.run(Host().loop())
