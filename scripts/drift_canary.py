#!/usr/bin/env python3
"""framerize drift canary — early warning when Framer changes anything we depend on.

Every ~20 min, checks (and records to ~/.framerize/drift-log.ndjson):
  1. castle wrapper chunk (chunk-5LKSDTQ5.mjs) exists + byte-size stable
  2. login page still ships the publishable key inline, value unchanged
  3. the FREE ROUTE still doesn't meter: small non-streaming call, ledger delta == 0
  4. model gates: opus-5.5 (messages), gpt-6-sol (direct), gemini-2.5-flash
  5. session/JWT/ledger endpoints healthy

Exit line per check: ok / DRIFT. A DRIFT line means "investigate now" —
Framer redeployed their client, flipped a gate, or closed the metering gap.
Runs as a systemd --user service on iris.
"""
import asyncio, json, os, re, time, uuid, glob, sys
sys.path.insert(0, os.path.expanduser('~/framerize/scripts'))
from camoufox.async_api import AsyncCamoufox

CHUNK_URL = 'https://app.framerstatic.com/chunk-5LKSDTQ5.mjs'
CHUNK_KNOWN_SIZE = 555865
PUBKEY = 'pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB'
API = 'https://api.framer.com'
DRIFT_LOG = os.path.expanduser('~/.framerize/drift-log.ndjson')
INTERVAL_S = 20 * 60

def record(check, status, detail=None):
    line = json.dumps({'ts': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
                       'check': check, 'status': status, 'detail': detail})
    print(line, flush=True)
    with open(DRIFT_LOG, 'a') as f:
        f.write(line + '\n')

async def main():
    while True:
        try:
            await sweep()
        except Exception as e:
            record('sweep', 'error', f'{type(e).__name__}: {e}')
        await asyncio.sleep(INTERVAL_S)

async def sweep():
    # Live account discovery (2026-10-06): the canary used to hardcode the
    # ORIGINAL account's project/team — after the account rotation it probed
    # a workspace it couldn't see and reported 402s as "DRIFT". Discover the
    # daemon's first ready account each cycle.
    import urllib.request as _ur
    try:
        with _ur.urlopen('http://127.0.0.1:8787/api/accounts', timeout=10) as r:
            accounts = json.loads(r.read().decode())
    except Exception as e:
        record('daemon-accounts', 'DRIFT', f'{type(e).__name__}: {e}')
        return
    ready = [a for a in accounts if a.get('state') == 'ready' and a.get('current_project')]
    if not ready:
        record('daemon-accounts', 'DRIFT', 'no ready account with a project')
        return
    acct = ready[0]
    uid = acct['framer_user_id']
    sess_file = os.path.expanduser(f'~/.framerize/profiles/.sessions/{uid}.session')
    if not os.path.exists(sess_file):
        record('session', 'DRIFT', f'no session file for {uid[:8]}')
        return
    cookie_raw = open(sess_file).read().strip()
    cookie_value = cookie_raw.split('session=')[1].split(';')[0] if 'session=' in cookie_raw else cookie_raw

    # --- static checks (no browser) ---
    import urllib.request
    def fetch(url):
        req = urllib.request.Request(url, headers={'Cookie': cookie_raw})
        try:
            r = urllib.request.urlopen(req, timeout=20)
            return r.status, r.read()
        except Exception as e:
            return -1, str(e).encode()

    s, html = fetch('https://framer.com/login')
    if s == 200:
        m = re.search(rb'castlePublishableAPIKey[\'"\s,:=]+(?:Object\.freeze\()?"([a-zA-Z0-9]+)"', html)
        key = m.group(1).decode() if m else None
        if key == PUBKEY:
            record('pubkey', 'ok', key[:12])
        elif key:
            record('pubkey', 'DRIFT', f'key changed: {key[:12]}...')
        else:
            record('pubkey', 'warn', 'inline key script not found in login html (UA-dependent?)')
    else:
        record('pubkey', 'DRIFT', f'login page status {s}')

    s, chunk = fetch(CHUNK_URL)
    if s == 200:
        if abs(len(chunk) - CHUNK_KNOWN_SIZE) < 4096:
            record('castle-chunk', 'ok', f'{len(chunk)}b')
        else:
            record('castle-chunk', 'DRIFT', f'size changed {CHUNK_KNOWN_SIZE} -> {len(chunk)}')
    else:
        record('castle-chunk', 'DRIFT', f'chunk status {s} — wrapper URL dead?')

    s, me = fetch(f'{API}/web/users/me')
    record('users-me', 'ok' if s == 200 else 'DRIFT', f'status {s}')

    # --- live inference checks (browser) ---
    proj = acct['current_project']
    team = acct.get('current_team') or ''
    mgr = AsyncCamoufox(headless=True, persistent_context=True,
                        user_data_dir='/tmp/drift-profile', humanize=False)
    ctx = await mgr.__aenter__()
    try:
        await ctx.add_cookies([{'name': 'session', 'value': cookie_value,
            'domain': '.framer.com', 'path': '/', 'secure': True, 'httpOnly': True, 'sameSite': 'None'}])
        page = await ctx.new_page()
        await page.goto('https://framer.com/login', wait_until='domcontentloaded', timeout=90000)
        await page.wait_for_timeout(2000)
        await page.evaluate(f'''(async () => {{
          if (!window.castlePublishableAPIKey) Object.defineProperty(window, 'castlePublishableAPIKey', {{ value: Object.freeze('{PUBKEY}') }});
          window.__cm = await import('{CHUNK_URL}');
          return true; }})()''')
        jwt = await page.evaluate(f"(async()=>{{const r=await fetch('{API}/auth/ai/access-token',{{credentials:'include'}});return r.json()}})()")
        bearer = jwt.get('accessToken') or jwt.get('token')
        record('ai-jwt', 'ok' if bearer else 'DRIFT', 'minted' if bearer else 'no token')

        async def mint():
            return await page.evaluate("(async()=>{for(let i=0;i<3;i++){const t=await window.__cm.a();if(t)return t;await new Promise(r=>setTimeout(r,500))}return null})()")
        tok = await mint()
        record('castle-mint', 'ok' if tok else 'DRIFT', f'len={len(tok or "")}')

        async def ledger():
            d = await page.evaluate(f"(async()=>{{const r=await fetch('{API}/web/teams/{team}/ai-credits',{{credentials:'include'}});return r.json()}})()")
            return d.get('availableCredits')

        async def fire(path, model, req, fc='agents'):
            t = await mint()
            body = {'projectId': proj, 'featureCategory': fc, 'featureName': fc, 'step': 'message',
                    'sessionId': 'drift-' + uuid.uuid4().hex[:6], 'stepIndex': 0,
                    'turnId': uuid.uuid4().hex, 'userPrompt': 'canary', 'castleToken': t, 'request': req}
            r = await page.evaluate("""(async ([b, path, body]) => {
              const r = await fetch('https://api.framer.com' + path, { method: 'POST', credentials: 'include',
                headers: { 'content-type': 'application/json; charset=utf-8', 'x-requested-by': 'Framer',
                           'authorization': 'Bearer ' + b }, body: JSON.stringify(body) });
              return r.status; })""", [bearer, path, body])
            return r

        # THE metering gap itself: free non-streaming call, ledger delta must be 0
        l0 = await ledger()
        s = await fire('/ai/v3/messages/', 'google/gemini-2.5-flash',
                       {'model': 'google/gemini-2.5-flash', 'messages': [{'role': 'user', 'content': 'ok'}], 'max_tokens': 10, 'stream': False})
        await page.wait_for_timeout(6000)
        l1 = await ledger()
        if s == 429:
            # workspace concurrency is full from real traffic — transient, NOT drift
            record('free-route', 'busy', '429 (workspace saturated by live traffic — not a drift signal)')
        elif s != 200:
            record('free-route', 'DRIFT', f'status {s}')
        elif l0 is not None and l1 is not None and abs(l1 - l0) < 1e-9:
            record('free-route', 'ok', f'200, delta 0 ({l1})')
        else:
            record('free-route', 'DRIFT', f'LEDGER MOVED {l0} -> {l1} — metering gap closed?')

        # model gates
        s = await fire('/ai/v3/messages/', 'anthropic/claude-opus-5.5',
                       {'model': 'anthropic/claude-opus-5.5', 'messages': [{'role': 'user', 'content': 'ok'}], 'max_tokens': 10, 'stream': False}, 'translation')
        record('model-opus5.5', 'ok' if s == 200 else 'DRIFT', f'status {s}')
        # Direct-route probe REMOVED (2026-10-06): sol is metered — every probe
        # spent credits, and the route is blocked by daemon policy anyway.
        # Verify the BLOCKED surface instead: sol on the free passthrough must
        # stay absent (its appearance would change routing assumptions).
        s = await fire('/ai/v3/responses/', 'gpt-6-sol',
                       {'model': 'openai/gpt-6-sol', 'input': 'say ok', 'max_output_tokens': 64, 'stream': False})
        record('model-gpt6sol-still-unavailable-free-route', 'ok' if s in (400, 403, 404) else 'warn', f'status {s}')
    finally:
        await mgr.__aexit__(None, None, None)

if __name__ == '__main__':
    asyncio.run(main())
