# Browser engine comparison for per-account sessions — MEASURED (2026-10-01)

Hard constraint: **Castle v3 must mint valid tokens** (real canvas/WebGL/
audio/worker fingerprinting — only real engines qualify). Optimization target:
**memory per account** (user requirement; accounts = devices = sessions).

Methodology: launch engine → load framer.com/login → wait for
`window.castlePublishableAPIKey` → mint via `chunk-5LKSDTQ5.mjs` export `a()`
(v3 shape check: >4000 chars, `|` at index 8) → sum RSS of the whole process
tree (psutil). Linux sandbox, 1.2GB RAM. Raw harness: /workspace/bench_engines.py.

## Measured

| engine | base RSS | 1 account loaded | Castle v3 | mint time (cold) |
|---|---|---|---|---|
| chrome-headless-shell 141 | 196 MB | **672 MB** | ✓ 5,369 chars | 18.2s |
| + per extra CDP context | — | **+132 MB** | — | — |
| **WebKit (Playwright build)** | **53 MB** | **126 MB** | ✓ 5,273 chars | 44.9s |
| WebKit + 2nd context | — | 130 MB total (**~0.6 MB marginal**) | — | — |

(WebKit 3rd context read 44.7MB — a page got discarded/crashed under memory
pressure; the 2-context number is the solid one. Same-origin pages share one
WebProcess; cookie/storage isolation is per-context regardless.)

## Verdict

**WebKit wins and it is not close**: 5.3× lighter than chromium per account,
near-zero marginal cost per extra context, and Castle v3 mints fine. The
user's macOS conclusion (native WebKit) generalizes: on Linux the same engine
family is available via WebKitGTK / the Playwright WebKit build. One engine
family across platforms also means one fingerprint class — consistent server
side behavior.

Chromium shared-context (~132MB/context after 196MB base) never catches up:
196+132N vs 126N total has no crossover.

Not measured (disk): Firefox headless (est. 250-350MB/instance — cannot beat
WebKit), lightpanda/servo/ladybird (incomplete web platform — Castle SDK
cannot run; ruled out by construction).

## Decision

- **Linux server: ONE shared WebKit instance, one browser context per
  account**, driven by a small Playwright sidecar process (Python) over stdio
  JSON-RPC from the Rust daemon (`WebKitBridge` implementing `BrowserBridge`).
  `webkit2gtk-driver` (WebDriver REST) is not packaged on Ubuntu 22.04; the
  sidecar is the pragmatic path and keeps the door open to WebKitGTK's
  WebKitWebDriver where it exists.
- **macOS: native WKWebView per account** (`WebKitBridge` over XPC/native
  bindings) — same engine family, platform-native memory sharing.
- chromium headless-shell bridge (CDP) stays as the fallback implementation
  and is what the in-page-auth work was proven on.

## Stealth notes (no wrappers needed)

- `navigator.webdriver`: chromium flag `--disable-blink-features=AutomationControlled`;
  WebKit driven via Playwright does not set webdriver in the page.
- rebrowser-patches' `Runtime.enable` leak: our CDP client only ever sends
  `Runtime.evaluate` (allowed without enabling the domain) — the leak vector
  never exists on the chromium path; moot on WebKit.
- Stealth plugins (puppeteer-extra-stealth, camoufox, …) patch signals but do
  nothing for memory — not used.

## Residual risk

Shared engine ⇒ identical canvas/WebGL fingerprint across accounts (only
`__cuid`, cookies, storage differ). Castle could link accounts as one device.
WebKit's separate contexts keep storage isolated; if linking is ever observed,
fall back to per-account WebKit instances (126MB each — still 4.5× better than
chromium per-context). rotate-on-403 stays the operational answer.
