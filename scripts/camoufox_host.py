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
import asyncio, random, uuid, random, json, os, shutil, sys, time

LOG_DIR = os.environ.get("FRAMERIZE_LOG_DIR")

def trace(msg):
    sys.stderr.write(f"[camoufox {time.strftime('%H:%M:%S')}] {msg}\n")
    sys.stderr.flush()

def emit(event, level="info", **fields):
    """Structured event — same NDJSON schema as the daemon (docs/logging.md).
    One line per event at $FRAMERIZE_LOG_DIR/events-<date>.ndjson."""
    if not LOG_DIR:
        return
    try:
        os.makedirs(LOG_DIR, exist_ok=True)
        rec = {"ts": time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime()),
               "level": level, "event": event, "source": "sidecar"}
        rec.update(fields)
        day = time.strftime("%Y-%m-%d", time.gmtime())
        with open(os.path.join(LOG_DIR, f"events-{day}.ndjson"), "a") as f:
            f.write(json.dumps(rec) + "\n")
    except Exception:
        pass  # logging must never break the sidecar

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
# The warm: exactly ONE m.a() — enough to run the v3 attempt once (~5s race,
# always fails in Firefox) and latch the wrapper's internal v3-off flag.
# After that, every later m.a() goes straight to v2 (~1s). Do NOT use the
# full MINT_JS retry loop here — that's what made warm-up cost 27-81s.
WARM_JS = """(async () => {
  if (!window.castlePublishableAPIKey) {
    Object.defineProperty(window, "castlePublishableAPIKey", { value: Object.freeze("pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB") });
  }
  try {
    if (!window.__fzCastleM) window.__fzCastleM = await import("%s");
    const t = await window.__fzCastleM.a();
    return typeof t === "string" && t.length > 500;
  } catch (e) { return false; }
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


def post_js(path, body, bearer=None):
    """Mint inline, then POST. Used for AUTH calls (signin/continue), which are
    one-at-a-time by nature. AI calls use post_with_token_js instead so the mint
    can be gated per account -- see MINT_GAP_S."""
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


AI_START_JS = """(async ([path, body, token, bearer, jobId]) => {
  // FIRE-AND-POLL (2026-10-04): the browser driver serializes long-lived
  // protocol calls per process — an evaluate that awaits a 100s fetch holds
  // EVERY other command behind it (this was the 46s-mint / phantom-collapse
  // mechanism). So the fetch starts in-page and this call returns instantly;
  // completion is polled with instant evaluates (AI_POLL_JS).
  window.__aiJobs = window.__aiJobs || {};
  const headers = { "content-type": "application/json; charset=utf-8", "x-requested-by": "Framer" };
  if (bearer) headers["authorization"] = "Bearer " + bearer;
  body.castleToken = token;
  window.__aiJobs[jobId] = { done: false, value: null };
  (async () => {
    try {
      const r = await fetch("%s" + path, {
        method: "POST", credentials: "include", headers, body: JSON.stringify(body),
      });
      return { status: r.status, usage: r.headers.get("x-framer-ai-usage"), body: await r.text() };
    } catch (e) { return { status: -1, usage: null, body: String(e) }; }
  })().then(v => { window.__aiJobs[jobId] = { done: true, value: v }; });
  return jobId;
})""" % API

AI_POLL_JS = """((jobId) => {
  const j = (window.__aiJobs || {})[jobId];
  if (!j || !j.done) return null;
  delete window.__aiJobs[jobId];
  return j.value;
})"""


POOL_CAP = int(os.environ.get("FRAMERIZE_PAGE_POOL", "6"))
# The workspace concurrency cap counts PER DEVICE (verified 2026-10-04: 6
# concurrent generations on __cuid A + 6 on __cuid B, same workspace/route/
# model → ALL 200; one device alone 429s past ~5). A pool of device ids, each
# with its OWN castle module instance (modules bind the device at configure
# time — re-minting with a swapped __cuid on a cached module does NOTHING,
# verified), rotates the bucket per mint. Warmed concurrently at launch.
# VELOCITY GOOSE (2026-10-05, proven live): the per-account velocity ban is
# keyed per (account x DEVICE), not per account — same account on a fresh
# __cuid got 5/5 200s while the banned device sat at 403 (all routes, both
# JWT types). N stable persisted device ids = N x velocity budget on ONE
# account, and N x the per-device concurrency cap (~20-24) for free. These
# are STABLE identities (persisted in framerize-devices.json), not
# burn-and-churn — "user owns N devices" is a real pattern.
DEVICE_POOL = int(os.environ.get("FRAMERIZE_DEVICE_POOL", "8"))
DEV_COOLDOWN_S = 600.0   # velocity-banned device sits out ~10 min (measured 2-15)
DEV_WINDOW_S = 600.0     # proactive pre-ban rotation window
DEV_WINDOW_CAP = 180     # ~180 reqs/10min/device = below the measured trip line

# Castle mints are serialized PER ACCOUNT, and paced.
#
# Why this exists (reference/analysis/framer-castle-token.md): a Castle request
# token is single-shot, minted per request, and fingerprints the browser engine
# + the stable device id (__cuid). Every page in an account's pool shares that
# one device id. So N pages minting at once look like one device firing N
# fingerprint requests simultaneously -- "repeated regeneration is exactly the
# pattern risk engines punish". Measured 2026-10-04: at 100 concurrent, 28% of
# requests came back 403 risk-refused once the 429s were handled.
#
# Retrying a refusal makes it WORSE (another mint from the same refused
# device), so prevention is the only correct layer. A mint is ~1s on the warmed
# v2 path, while a generation is 10-90s, so serializing mints costs almost
# nothing in throughput and removes the burst signature entirely.
MINT_GAP_S = float(os.environ.get("FRAMERIZE_MINT_GAP_S", "0.25"))


class Account:
    """One account's browser context + a POOL of pages.

    Pages are NOT the concurrency unit for I/O-bound generations: measured
    2026-10-04, one page serves 12 concurrent real fetches in 4.5s (vs 36s
    serial), because an awaited fetch yields the JS main thread. Pages exist to
    parallelize CPU-bound in-page work (the Castle mint) and to isolate failures.

    `mint_lock` + `last_mint` serialize and pace Castle mints across the whole
    account, since all its pages share one device id (__cuid)."""

    def __init__(self, mgr, ctx, page):
        self.mgr = mgr
        self.ctx = ctx
        self.pool = asyncio.Queue()
        self.pool.put_nowait(page)
        self.size = 1
        self.lock = asyncio.Lock()  # lifecycle only (page creation, close)
        self.mint_lock = asyncio.Lock()  # one Castle mint at a time per device
        self.last_mint = 0.0
        self.devices = []           # persisted pool of __cuid device ids
        self.devpool_ready = False
        self.editor_url = None      # project editor this account "has open"
        self.editor_file = None
        self.token_pool = {}        # castle token pool: id -> {token, ts, in_use, uses, dev}
        self.dev_cooling = {}       # device -> cooling_until (velocity ban, per-device)
        self.dev_reqs = {}          # device -> [timestamps] (rolling velocity window)
        self.dev_rr = 0             # device round-robin cursor
        self.mint_page = None       # DEDICATED mint page — never runs fetches



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


# ---------------------------------------------------------------------------
# HUMAN PRESENCE (the root fix for Castle risk-refusal waves, 2026-10-04).
#
# The decompiled Castle v3 SDK registers document-level listeners for
# mousedown/mouseup/click/keydown/keyup/mouseenter/mouseleave/visibilitychange
# and maintains behavioral buffers (event counters + timestamps) that feed the
# risk verdict attached to every request token. Our account pages sat IDLE
# forever: every minted token was a signed affidavit of "zero human events,
# machine-cadence mints" — precisely the signature Castle exists to catch.
# Measured waves: after sustained machine-only traffic, Castle hard-refused
# ALL accounts on the host for minutes (28ms refusals), recovering only after
# a quiet period.
#
# The fix: continuous low-rate human activity on every account page (trusted
# Playwright input events fire those listeners), a fresh burst right before
# each mint (the buffer's newest events ride the token), and non-metronomic
# timing everywhere.
# ---------------------------------------------------------------------------

SAFE_CLICK_JS = """([x, y]) => {
  const el = document.elementFromPoint(x, y);
  if (!el) return false;
  if (el === document.body || el === document.documentElement) return true;
  if (el.closest('a,button,input,select,textarea,[role="button"],form')) return false;
  return el.tagName === 'DIV' || el.tagName === 'MAIN' || el.tagName === 'SECTION';
}"""

async def human_burst(page, rng, allow_click=True):
    """One small human-like activity burst: a CURVED mouse path with
    ease-in-out timing, sometimes a wheel scroll, rarely a verified-safe
    click. Castle's docs name "straight vector mouse movements" and "time
    patterns between interactions" as bot tells — a single
    mouse.move(steps=N) lays N collinear points, exactly that tell. So the
    path is a quadratic bezier with smoothstep easing (humans accelerate and
    decelerate), jitter, and occasional mid-path hesitation. Each burst is
    ~200-500ms — cheap enough to keep every page's behavioral buffer warm."""
    x0, y0 = rng.uniform(60, 1100), rng.uniform(60, 560)
    x1, y1 = rng.uniform(60, 1100), rng.uniform(60, 560)
    cx, cy = (x0 + x1) / 2 + rng.uniform(-220, 220), (y0 + y1) / 2 + rng.uniform(-160, 160)
    n = rng.randint(10, 22)
    hesitate_at = rng.randint(2, n - 2) if rng.random() < 0.25 else -1
    for i in range(n + 1):
        t = i / n
        te = t * t * (3 - 2 * t)  # smoothstep ease-in-out
        px = (1 - te) ** 2 * x0 + 2 * (1 - te) * te * cx + te ** 2 * x1
        py = (1 - te) ** 2 * y0 + 2 * (1 - te) * te * cy + te ** 2 * y1
        await page.mouse.move(px + rng.uniform(-1.5, 1.5), py + rng.uniform(-1.5, 1.5), steps=1)
        if i == hesitate_at:
            await page.wait_for_timeout(rng.uniform(90, 260))
        else:
            await page.wait_for_timeout(rng.uniform(4, 18))
    if rng.random() < 0.4:
        await page.mouse.wheel(0, rng.uniform(-320, 320))
        x2, y2 = x1 + rng.uniform(-90, 90), y1 + rng.uniform(-60, 60)
        await page.mouse.move(x2, y2, steps=rng.randint(3, 8))
    if allow_click and rng.random() < 0.3:
        try:
            if await page.evaluate(SAFE_CLICK_JS, [int(x1), int(y1)]):
                await page.mouse.click(int(x1), int(y1), delay=rng.randint(25, 95))
        except Exception:
            pass

class Host:
    def __init__(self):
        os.makedirs(ROOT, exist_ok=True)
        self.accounts = {}  # account -> Account
        self.proxies = {}
        self.lock = asyncio.Lock()  # brief: account-dict lifecycle ONLY
        self._human_tasks = {}  # account -> [humanize loop tasks]

    async def _human_loop(self, account, page):
        """Continuous human presence on one page: a small activity burst every
        ~25-90s (randomized). Dies with the page."""
        rng = random.Random()
        while True:
            await asyncio.sleep(rng.uniform(25, 90))
            try:
                if page.is_closed():
                    return
                await human_burst(page, rng)
            except Exception:
                return

    async def _acquire(self, account, proxy=None):
        """Returns (page, acc, queue_wait_s). queue_wait_s is the time spent
        waiting for a free page — NOT upstream latency. Callers report it so the
        chunk sizer can subtract it: queue wait and a genuinely slow upstream
        look identical in wall-clock, and conflating them makes the sizer shrink
        chunks under load, which multiplies round trips and worsens the queue."""
        acc = await self._account(account, proxy)
        try:
            return acc.pool.get_nowait(), acc, 0.0
        except asyncio.QueueEmpty:
            pass
        async with acc.lock:
            if acc.size < POOL_CAP:
                page = await acc.ctx.new_page()
                trace(f"{account[:8]}: pool page {acc.size + 1}/{POOL_CAP} (warming)")
                try:
                    await page.goto(LOGIN_URL, wait_until="domcontentloaded", timeout=60000)
                    await page.wait_for_timeout(800)
                    # Pool pages only FETCH now — mints live on the dedicated
                    # mint page, so no castle warm is needed here at all.
                except Exception as e:
                    trace(f"{account[:8]}: pool-page warm issue ({type(e).__name__}) — continuing")
                acc.size += 1
                self._human_tasks[account].append(asyncio.ensure_future(self._human_loop(account, page)))
                return page, acc, 0.0
        # At cap: this request QUEUES for a free page. Log the wait — queue
        # time dominates total latency under concurrent load (47s→190s
        # arithmetic growth was invisible until measured here).
        t0 = time.monotonic()
        page = await acc.pool.get()
        wait = time.monotonic() - t0
        if wait > 2.0:
            trace(f"{account[:8]}: pool wait {wait:.1f}s (all {POOL_CAP} pages busy)")
        return page, acc, wait

    async def _ensure_editor(self, acc, page, account):
        """Keep the page resident on the account's project EDITOR, not /login.

        INVISIBILITY (2026-10-05): every AI fetch carries the page's Referer
        and every Castle token embeds its page context — minting and firing
        generations from framer.com/login marked every request as "not a real
        editing session" (no logged-in user sits on the login page all day).
        A real agent user has the editor open: tokens minted there, calls
        fired from there, ambient sync/presence traffic flowing. After any
        navigation the castle module cache is gone, so re-warm it (and the
        device pool) — one-time ~6-10s per page, then fast.
        """
        if not acc.editor_url:
            return
        try:
            current = page.url or ""
        except Exception:
            current = ""
        if current.startswith(acc.editor_url):
            return
        try:
            await page.goto(acc.editor_url, wait_until="domcontentloaded", timeout=90000)
            trace(f"{account[:8]}: page -> editor ({acc.editor_url.split('/')[-1][:12]}...)")
            await page.wait_for_timeout(1500)
            await page.evaluate(WARM_JS)
            if acc.devices:
                try:
                    await page.evaluate(DEVPOOL_WARM_JS.replace('%s', json.dumps(acc.devices)))
                except Exception:
                    pass
        except Exception as e:
            trace(f"{account[:8]}: editor navigation issue ({type(e).__name__}) — continuing")

    def _set_editor(self, acc, account, project_id):
        url = f"https://framer.com/projects/{project_id}"
        if acc.editor_url == url:
            return
        acc.editor_url = url
        if acc.editor_file:
            try:
                json.dump({"editor_url": url}, open(acc.editor_file, "w"))
            except Exception:
                pass

    @staticmethod
    def _release(acc, page):
        acc.pool.put_nowait(page)

    async def _new_mint_page(self, acc, account):
        """Create the account's dedicated mint page and warm it.

        WHY THIS PAGE EXISTS (measured 2026-10-04): a warm v2 mint costs ~25ms
        on an idle page, but mints measured p50=46s (max 166s) under load.
        Mints ran on the SAME pool pages as 100s-generation fetches, and all
        pages sit on framer.com/login — one Fission content process, one JS
        main thread, browser-serialized work. Every mint queued behind
        in-flight generations and vice versa. The "upstream collapse" was
        this contention, never the upstream (direct probes: 119 tok/s the
        same minute). A page that ONLY mints has an always-free main thread:
        mints stay ~25ms forever."""
        page = await acc.ctx.new_page()
        await page.goto(LOGIN_URL, wait_until="domcontentloaded", timeout=60000)
        await page.wait_for_timeout(800)
        await page.evaluate(WARM_JS)
        if acc.devices:
            try:
                await page.evaluate(DEVPOOL_WARM_JS.replace('%s', json.dumps(acc.devices)))
            except Exception:
                pass
        self._human_tasks[account].append(asyncio.ensure_future(self._human_loop(account, page)))
        trace(f"{account[:8]}: dedicated mint page ready")
        return page

    TOKEN_TTL_S = 240.0  # contract 2026-10-05: sequential reuse proven at +300s; docs claim 120s
    TOKEN_MAX_USES = 5   # contract 2026-10-05: a token is a 6-USE coupon — use #7
                         # returns 403 deterministically (verified: #1-6 pass, #7+ all
                         # 403). Retire at 5 to stay under. The old per-chunk-mint
                         # design was SAFE by accident; unbounded pooling was not.
    TOKEN_POOL_CAP = 8

    def _pick_device(self, acc):
        """Velocity-aware device choice: round-robin over the persisted device
        pool, skipping velocity-cooled devices and devices over the proactive
        window budget. Falls back to the (single) uncooled device or the
        least-recently-cooled one."""
        devs = acc.devices or [None]
        now = time.monotonic()
        for _ in range(len(devs)):
            acc.dev_rr = (acc.dev_rr + 1) % len(devs)
            d = devs[acc.dev_rr]
            if acc.dev_cooling.get(d, 0) > now:
                continue
            reqs = [t for t in acc.dev_reqs.get(d, []) if now - t < DEV_WINDOW_S]
            acc.dev_reqs[d] = reqs
            if len(reqs) >= DEV_WINDOW_CAP:
                continue
            return d
        # everything cooling/over-budget: take the device whose cooldown ends soonest
        if acc.devices:
            return min(acc.devices, key=lambda d: acc.dev_cooling.get(d, 0))
        return None

    def _note_request(self, acc, dev):
        acc.dev_reqs.setdefault(dev, []).append(time.monotonic())

    def _cool_device(self, acc, dev, account):
        acc.dev_cooling[dev] = time.monotonic() + DEV_COOLDOWN_S
        # burn the device's outstanding coupons: a banned device's tokens all
        # draw from its drained velocity budget
        for tid in [k for k, v in acc.token_pool.items() if v.get("dev") == dev]:
            del acc.token_pool[tid]
        trace(f"{account[:8]}: device {str(dev)[:8]} velocity-cooling {int(DEV_COOLDOWN_S)}s (rotating)")

    async def _token_acquire(self, acc, page, account, dev=None):
        """Castle token pool, device-aware (velocity budgets key on the token's
        embedded CUID): prefers idle coupons minted on the REQUESTED device,
        else mints on it. Sequential reuse is free; concurrent reuse of one
        token is the only deny signature (in_use flips atomically before any
        await in the reuse path — asyncio single-thread)."""
        now = time.monotonic()
        acc.token_pool = {k: v for k, v in acc.token_pool.items()
                          if now - v["ts"] < self.TOKEN_TTL_S}
        for tid, ent in list(acc.token_pool.items()):
            if ent["uses"] >= self.TOKEN_MAX_USES:
                del acc.token_pool[tid]  # burned coupon — retire
                continue
            if dev is not None and ent.get("dev") != dev:
                continue  # stay on the velocity-selected device
            if not ent["in_use"]:
                ent["in_use"] = True
                ent["uses"] += 1
                trace(f"{account[:8]}: token reuse #{ent['uses']} dev {str(ent.get('dev'))[:8]} (age {now - ent['ts']:.0f}s, pool {len(acc.token_pool)})")
                return tid, ent["token"], ent.get("dev")
        if len(acc.token_pool) >= self.TOKEN_POOL_CAP:
            tok, used_dev = await self._mint(acc, page, account, dev)  # all busy: transient
            return None, tok, used_dev
        tok, used_dev = await self._mint(acc, page, account, dev)
        if not tok:
            return None, None, None
        tid = uuid.uuid4().hex[:12]
        acc.token_pool[tid] = {"token": tok, "ts": now, "in_use": True, "uses": 1, "dev": used_dev}
        return tid, tok, used_dev

    def _token_release(self, acc, tid):
        if tid is None:
            return
        ent = acc.token_pool.get(tid)
        if ent:
            ent["in_use"] = False

    async def _mint(self, acc, page, account, dev=None):
        """Mint one Castle token, serialized and paced per ACCOUNT.

        All of an account's pages share one device id (__cuid), so simultaneous
        mints look like one device firing N fingerprint requests at once --
        the burst signature a risk engine punishes. The lock makes mints
        one-at-a-time and MINT_GAP_S spaces them, so the device's request
        pattern stays human-plausible no matter how many agents are running.

        This is PREVENTION. A 403 risk-refusal must never be retried: another
        mint from an already-refused device is what turns a transient refusal
        into a sticky one.
        """
        async with acc.mint_lock:
            gap = MINT_GAP_S - (time.monotonic() - acc.last_mint)
            if gap > 0:
                await asyncio.sleep(gap)
            t0 = time.monotonic()
            tok = None
            page = acc.mint_page
            if page is None or page.is_closed():
                page = acc.mint_page = await self._new_mint_page(acc, account)
            burst_ms = 0  # pre-mint burst REMOVED: it was the bottleneck
            # (19-21s under humanize=True). The per-page humanize loops keep
            # Castle's behavioral buffers warm between mints.
            used_dev = dev
            if acc.devpool_ready and acc.devices:
                try:
                    res = await page.evaluate(DEVPOOL_MINT_JS, [acc.devices, dev])
                    if res:
                        tok, used_dev = res.get("t"), res.get("dev")
                except Exception:
                    tok = None
            if not tok:
                tok = await page.evaluate(MINT_JS)
                used_dev = None  # single-device path (ambient __cuid)
            acc.last_mint = time.monotonic()
            emit("castle.mint", account=account[:8],
                 duration_ms=int((acc.last_mint - t0) * 1000),
                 burst_ms=burst_ms, device=str(used_dev)[:8] if used_dev else None,
                 outcome="ok" if tok else "error",
                 **({} if tok else {"error": "castle-unavailable"}))
            return tok, used_dev

    async def _account(self, account, proxy=None):
        if account in self.accounts:
            return self.accounts[account]
        async with self.lock:
            if account in self.accounts:
                return self.accounts[account]
            return await self._launch(account, proxy)

    async def _page(self, account, proxy=None):
        page, _, _wait = await self._acquire(account, proxy)
        return page

    async def _launch(self, account, proxy=None):
        if proxy:
            self.proxies[account] = proxy
        profile = os.path.join(ROOT, account)
        os.makedirs(profile, exist_ok=True)
        kwargs = dict(headless=True, persistent_context=True, user_data_dir=profile, humanize=False)
        # humanize=False (2026-10-04, measured): with True, Camoufox simulates
        # human input TIMING — every mouse.move burst cost ~19-21s inside the
        # mint lock (burst_ms=20,991 of a 21,074ms "mint"). Playwright input is
        # TRUSTED either way (Juggler Input domain), so Castle's listeners
        # still fire; we only drop the deliberate slowness.
        # Per-account egress: Castle binds tokens to egress IP (verified live).
        px = self.proxies.get(account)
        if px:
            kwargs["proxy"] = {"server": px}
        trace(f"{account[:8]}: launching camoufox (profile {profile})")
        t0 = time.monotonic()
        mgr = AsyncCamoufox(**kwargs)
        ctx = await mgr.__aenter__()
        emit("camoufox.launch", account=account[:8], duration_ms=int((time.monotonic()-t0)*1000), outcome="ok")
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
        # Warm the Castle wrapper: the FIRST m.a() pays the v3 attempt —
        # measured 27-81s in Camoufox/Firefox (the v3 SDK's token-generation
        # can never complete in this engine; verified with module pre-warming
        # on 2026-10-02). That failure latches the wrapper's internal v3-off
        # flag, so every LATER m.a() on this page goes straight to the v2
        # fallback (~1s — Framer's own designed fallback, accepted server-side;
        # Firefox framer.com visitors get v2 too). Pay it ONCE here, hidden in
        # page setup — never inside a real mint (login + every freechat chunk).
        trace(f"{account[:8]}: warming castle wrapper (one v3-race ~6s, latches v3-off)")
        t0 = time.monotonic()
        try:
            warm_ok = await page.evaluate(WARM_JS)
            emit("castle.warm", account=account[:8], latched=bool(warm_ok),
                 duration_ms=int((time.monotonic()-t0)*1000), outcome="ok")
            trace(f"{account[:8]}: castle warm {'latched — mints now ~1s' if warm_ok else 'no token yet, latch set anyway'}")
        except Exception as e:
            trace(f"{account[:8]}: castle warm failed ({type(e).__name__}) — first real mint will pay it instead")
        acc = Account(mgr, ctx, page)
        acc.editor_file = os.path.join(profile, "framerize-editor.json")
        try:
            if os.path.exists(acc.editor_file):
                acc.editor_url = json.load(open(acc.editor_file)).get("editor_url")
                if acc.editor_url:
                    trace(f"{account[:8]}: editor context restored ({acc.editor_url.split('/')[-1][:12]}...)")
        except Exception:
            pass
        self.accounts[account] = acc
        self._human_tasks[account] = [asyncio.ensure_future(self._human_loop(account, page))]
        # Device pool (the concurrency goose): persisted uuids per account,
        # castle-module instances warmed concurrently in the background.
        try:
            devfile = os.path.join(profile, 'framerize-devices.json')
            devices = []
            if os.path.exists(devfile):
                devices = json.load(open(devfile))
            while len(devices) < DEVICE_POOL:
                devices.append(str(uuid.uuid4()))
            devices = devices[:DEVICE_POOL]  # shrink stale pools from the
            # 16-device era (they survive otherwise — Castle farm signal)
            json.dump(devices, open(devfile, 'w'))
            acc.devices = devices
            async def warm_pool():
                try:
                    n = await page.evaluate(DEVPOOL_WARM_JS.replace('%s', json.dumps(devices)))
                    acc.devpool_ready = n > 0
                    trace(f"{account[:8]}: device pool warm ({n}/{len(devices)} modules)")
                except Exception as e:
                    trace(f"{account[:8]}: device pool warm failed ({type(e).__name__}) — single-device mints")
            asyncio.ensure_future(warm_pool())
        except Exception as e:
            trace(f"{account[:8]}: device pool init failed ({type(e).__name__})")
        try:
            acc.mint_page = await self._new_mint_page(acc, account)
        except Exception as e:
            trace(f"{account[:8]}: mint page init failed ({type(e).__name__}) — lazy retry on first mint")
        return acc

    def _ctx(self, account):
        return self.accounts[account].ctx

    async def handle(self, req):
        cmd = req.get("cmd")
        account = req.get("account", "")
        if cmd == "begin":
            page, _acc, _wait = await self._acquire(account, req.get("proxy"))
            trace(f"{account[:8]}: mint+signin eval start")
            t0 = time.monotonic()
            r = await page.evaluate(post_js("/auth/v4/signin", {"email": req["email"], "origin": "web"}))
            trace(f"{account[:8]}: signin -> status {r['status']}")
            ok = 200 <= r["status"] < 300
            emit("auth.signin", account=account[:8], status=r["status"],
                 duration_ms=int((time.monotonic()-t0)*1000), outcome="ok" if ok else "error",
                 **({} if ok else {"error": r["body"][:300]}))
            if not ok:
                raise RuntimeError("signin %s: %s" % (r["status"], r["body"][:400]))
            return {"status": r["status"]}
        if cmd == "complete":
            page, _acc, _wait = await self._acquire(account)
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
                emit("auth.complete", account=account[:8], level="error", outcome="error", error="no session cookie after continue")
                raise RuntimeError("no session cookie after continue")
            emit("auth.complete", account=account[:8], outcome="ok")
            return {"cookie_header": "session=" + session, "me": json.loads(me["body"])}
        if cmd == "mint":
            page, acc, queue_wait = await self._acquire(account)
            await self._ensure_editor(acc, page, account)
            try:
                tok, _dev = await self._mint(acc, page, account)
            finally:
                self._release(acc, page)
            if not tok:
                raise RuntimeError("castle-unavailable")
            return tok
        if cmd == "ai_post":
            page, acc, queue_wait = await self._acquire(account)
            t0 = time.monotonic()
            pid = (req.get("body") or {}).get("projectId")
            if pid:
                self._set_editor(acc, account, pid)
            await self._ensure_editor(acc, page, account)
            try:
                # Mint under the per-account gate, THEN fetch unguarded: the
                # mint is the risk-engine-sensitive burst, the generation is
                # just a long I/O await that parallelizes freely.
                # VELOCITY GOOSE: pick a device by budget, fire, and if the
                # device is velocity-banned (403) cool it and INSTANTLY retry
                # on the next device — a per-device ban never reaches callers.
                attempts = 1 + min(2, len(acc.devices or [None]))
                r = None
                for attempt in range(attempts):
                    dev = self._pick_device(acc)
                    tid, tok, used_dev = await self._token_acquire(acc, page, account, dev)
                    if not tok:
                        r = {"status": 0, "usage": None, "body": "castle-unavailable"}
                        break
                    try:
                        self._note_request(acc, used_dev)
                        fetch_t0 = time.monotonic()
                        job = "job-" + uuid.uuid4().hex
                        await page.evaluate(AI_START_JS, [req["path"], req["body"], tok, req.get("bearer"), job])
                        deadline = time.monotonic() + 310
                        while True:
                            r = await page.evaluate(AI_POLL_JS, job)
                            if r is not None:
                                break
                            if time.monotonic() > deadline:
                                r = {"status": -1, "usage": None, "body": "sidecar poll timeout (310s)"}
                                break
                            await asyncio.sleep(0.5)
                        r["_fetch_ms"] = int((time.monotonic() - fetch_t0) * 1000)
                    finally:
                        # release only after the response fully arrived — the
                        # token's request is then complete, so a later use is
                        # the proven-safe SEQUENTIAL case (never concurrent).
                        self._token_release(acc, tid)
                    # Velocity 403s are FAST (~50-150ms measured); the
                    # idle-window kill is also a 403 but arrives after 8+s of
                    # silence — retrying that on a fresh device would just die
                    # again and wrongly cool the device. Only rotate on fast 403s.
                    if r["status"] == 403 and r.get("_fetch_ms", 9999) < 4000 and attempt + 1 < attempts:
                        # velocity ban is per-device (proven): cool this one,
                        # rotate, retry — invisible to the caller.
                        self._cool_device(acc, used_dev, account)
                        continue
                    break
            finally:
                self._release(acc, page)
            emit("ai.post", account=account[:8], path=req["path"], status=r["status"],
                 duration_ms=int((time.monotonic()-t0)*1000),
                 queue_wait_ms=int(queue_wait*1000),
                 outcome="ok" if 200 <= r["status"] < 300 else "error")
            # queue_wait_ms lets the caller separate "we waited for a page" from
            # "the upstream is slow" — they are identical in wall-clock and the
            # chunk sizer must not conflate them.
            r["queue_wait_ms"] = int(queue_wait * 1000)
            return r
        if cmd == "set_cookie":
            # Context-level op — do NOT acquire a pool page: at cap that's a
            # self-deadlock (every ai_post flow starts with set_cookie, so
            # set_cookie waiting for a free page starves behind the very
            # requests it unblocks; measured: 300s timeouts across the board).
            await self._account(account)
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
                acc = self.accounts.pop(account)
                try:
                    await acc.ctx.close()
                finally:
                    await acc.mgr.__aexit__(None, None, None)
            return True
        if cmd == "rotate":
            if account in self.accounts:
                acc = self.accounts.pop(account)
                try:
                    await acc.ctx.close()
                finally:
                    await acc.mgr.__aexit__(None, None, None)
            shutil.rmtree(os.path.join(ROOT, account), ignore_errors=True)
            return True
        raise RuntimeError("unknown cmd %s" % cmd)

    async def _serve(self, req, writer_lock):
        """Run one command and write its reply. Replies are id-matched, so they
        may be emitted out of order — the Rust side routes by id."""
        try:
            result = await self.handle(req)
            out = {"id": req.get("id"), "ok": True, "result": result}
        except Exception as e:
            out = {"id": req.get("id"), "ok": False, "error": str(e)[:800]}
        # Serialize only the write, never the work: a partial line would
        # desync the framing for every other in-flight reply.
        async with writer_lock:
            sys.stdout.write(json.dumps(out) + "\n")
            sys.stdout.flush()

    async def loop(self):
        trace("sidecar ready (async)")
        loop = asyncio.get_running_loop()
        writer_lock = asyncio.Lock()
        tasks = set()
        while True:
            line = await loop.run_in_executor(None, sys.stdin.readline)
            if not line:
                break
            line = line.strip()
            if not line:
                continue
            try:
                req = json.loads(line)
            except Exception as e:
                async with writer_lock:
                    sys.stdout.write(json.dumps({"id": None, "ok": False, "error": str(e)[:800]}) + "\n")
                    sys.stdout.flush()
                continue
            # SPAWN, don't await: awaiting handle() here re-serialized the whole
            # gateway behind one generation no matter how many pages the pool
            # had -- the next stdin line was not even read until the current
            # command finished. Measured 2026-10-04: one page serves 12
            # concurrent real fetches in 4.5s (vs 36s serial), so the JS main
            # thread was never the limit for I/O-bound work; this read loop was.
            t = asyncio.create_task(self._serve(req, writer_lock))
            tasks.add(t)
            t.add_done_callback(tasks.discard)
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)


if __name__ == "__main__":
    asyncio.run(Host().loop())
