# Castle anti-abuse — how the token works and what "one account = one device" means

Sources: `chunk-5LKSDTQ5.mjs` (the app's Castle wrapper), live capture, observed
behavior in an automated browser.

## What Castle is

Castle.io is a device-fingerprint / bot-detection SDK. Framer requires a fresh
Castle **request token** on every sensitive call: login (`/auth/v4/signin`) and
every AI call (`castleToken` field in `/ai/v3/*` bodies).

## How the app generates it (exact code path)

`chunk-5LKSDTQ5.mjs` exports one function (used as `dB()` by the AI transport and
by the login bundle):

```js
// pseudocode of the module
async function getCastleToken() {
  const pk = window.castlePublishableAPIKey;          // set on every framer.com page
  // v3 path:
  const sdk = await import("https://app.framerstatic.com/castleSDKV3-LNGA36I3.mjs");
  const client = await sdk.configure({ pk, onError }); // cached singleton
  const v3 = await client.createRequestToken();        // 5 s timeout
  if (v3) return v3;
  // v2 fallback (bundled legacy SDK):
  return legacyCastle.configure({ pk }).createRequestToken();
}
```

- Token shape: `<clientId>|<payload>` — observed `cNz8rPYH|JABFNDhU…` (~5.9 KB).
- Tokens are single-shot and short-lived: mint one **per request**, never cache.
- Verified live: from a framer.com page, `await (await import("…/chunk-5LKSDTQ5.mjs")).a()`
  returns a working token. This is exactly what `reference/tools/framer-ai-console.js` does.
- The chunk hash (`5LKSDTQ5`) changes on deploys — resolve it dynamically
  (see "Resolving the Castle module" below) or treat it as a fragile-layer constant.

## Why a plain HTTP client cannot do this

`createRequestToken()` fingerprints the browser engine (canvas, WebGL, fonts,
timing, navigator…). There is no server-side shortcut: the token must be minted
inside a real browser context. **This is the reason Framerize needs one embedded
WebKit/WebView session per account** — not just for cookies, but because Castle
binds the account's risk profile to that engine's fingerprint.

## The device id: `__cuid` (critical for account stability)

Castle keys every request token to a stable visitor id stored as `__cuid` in BOTH
a `.framer.com` cookie AND localStorage (observed live: one UUIDv4, same value in
both stores). The token's 8-char prefix is stable per device; the payload is
unique per mint. If a webview's storage is wiped, Castle mints a NEW device id
and the account suddenly looks like a brand-new device — repeated regeneration
is exactly the pattern risk engines punish.

Framerize rule: generate `ManagedAccount.castle_device_id` ONCE at account
creation, persist it with the account record, and seed it into the webview
(cookie + localStorage) before the first token mint of every session
(`castle::SEED_DEVICE_SNIPPET`). Rotate it only as the deliberate
fresh-identity remediation for env-sticky 403s.

## "Accounts are like different devices"

Each Framerize account gets its own isolated WebKit session (own cookie jar, own
storage, own fingerprint surface). To Framer/Castle each one looks like a distinct,
stable device:

- keep the session **persistent** (same fingerprint over time builds trust);
- never share storage between accounts;
- mint Castle tokens only inside that account's own webview.

## Observed failure mode (important)

In a datacenter automation browser, **every** `/ai/v3/*` call — including the
Framer web app's own UI calls — returns HTTP 403 `{"error":{"message":"Forbidden"}}`,
while `/auth/ai/access-token` and `/web/teams/…/ai-credits` keep working. The UI
shows "Authentication error / your session might have expired". Root cause is the
risk engine (Castle score + invisible reCAPTCHA Enterprise, site key
`6LcRWAYqAAAAAO5gUOuCftzMzi6K1qwQJq5g3lxj`), not auth. Takeaways:

- 403 ≠ signed out. Never mark an account dead on 403; cool down and retry from
  its own browser session.
- A clean residential browser profile works (the capture account used AI normally
  ~23 h earlier from a real browser).

## Resolving the Castle module dynamically

1. Load the editor page; read
   `performance.getEntriesByType("resource")` for `chunk-*.mjs` containing the
   string `castleSDKV3` (fetch the few candidates and grep), or
2. hook `import()`/fetch once and capture what the app itself imports when it
   builds its first AI request — store the URL as a per-deploy constant.
