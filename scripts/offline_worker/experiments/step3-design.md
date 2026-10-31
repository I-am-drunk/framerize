# Step 3 design: what the no-JSDOM realm must fake, and what it cannot

Two facts settled before writing the realm:

1. node-canvas 3.2.3 was built here and run against the real Chrome's
   captured canvas probe. Output length 8610 versus Chrome's 10786, first
   difference at byte 69. The PNGs are not byte-identical. Any design that
   draws the canvas itself (node-canvas, headless shell) cannot reproduce
   a real Chrome fingerprint. Drawing is out.

2. The traced mint shows the SDK calls `canvas.getContext('2d')` 19 times
   and never calls `toDataURL` or `toBlob`. It reads pixels through the
   context (getImageData / measureText / isPointInPath style probes), so a
   pre-baked PNG answer is also not enough: the answer must be a working
   2D context whose pixel reads equal real Chrome's.

Those two together leave one honest route for canvas: run the actual probe
in the user's real Chrome once, capture the pixel-level answers, and have
the realm's fake context replay those captured answers per probe. That is
a cache of the user's own browser, refreshed when the browser changes.
It is still local, still no 6-9 GB resident browser, and invents nothing.
It is also the exact boundary of "supported": Castle documents mocked DOMs
as unsupported, so this whole design remains a compatibility shim, not a
sanctioned configuration. That risk is unchanged by how well it works.

Everything except canvas is simple data and is already captured:
user agent, platform, vendor, webdriver=false, screen 1440x1000,
colorDepth 24, hardwareConcurrency 4, languages en-US/en, dpr 1.

## Realm architecture (build list)

1. A plain `node:vm` context. No JSDOM anywhere. Global object gets:
   `window`, `self`, `globalThis` (same object), `document`, `navigator`,
   `screen`, `location`, `performance`, `crypto`, `localStorage`,
   `sessionStorage`, plus timers and TextEncoder/TextDecoder.
2. `navigator`: the captured right-hand column values, exposed as plain
   getters with the same property descriptors Chrome uses (own, on the
   Navigator prototype chain where the SDK expects them).
3. `document`: a minimal object supporting `createElement('canvas')` and
   `addEventListener`. Only the canvas branch does work; other tags return
   inert stubs with the property shapes the trace recorded.
4. Canvas: a fake 2D context whose drawing methods record operations and
   whose pixel-reading methods return the captured Chrome answers. Font
   measurement returns the captured widths for the three real fonts
   (Arial, Courier New, Times New Roman).
5. Event surface: `addEventListener`/`removeEventListener` that accept and
   store listeners without firing them, matching a headless session that
   sees no user activity. The trace shows the SDK tolerates silence.
6. Storage: empty Map-backed localStorage/sessionStorage with getItem,
   setItem, removeItem, and a `length` of 0, matching the captured state.

The mint path (wrapper -> configure -> createRequestToken -> collection)
stays identical to production; only the global environment changes.
Offline check: diff the collected 765-slot array against both the JSDOM
run and a real-browser run before any live request.

## Correction found while building: the production worker tells the truth

I measured the JSDOM default user agent (`jsdom/26.1.0`) and assumed the
production token claims to be Chrome. It does not. `dedicated_workers.mjs`,
`dedicated_runtime.mjs` and `resources.mjs` all pin the user agent to the
literal string `Framerize/0.1`. The browserless worker is deliberately
honest about being a Node program.

That reframes the 403. Framer is not being fooled and catching us. Framer
is being told, in the token's own user-agent field, "this did not come from
a browser," and Framer's policy is to require a browser. The 403 is a
policy decision, not a signal-matching failure we can tune around.

## Why the replay design stops here

Making Framer accept a no-browser token means making the token claim to be
real Chrome: Chrome's user agent, Chrome's canvas bytes, Chrome's screen,
Chrome's everything, with no browser behind it. That is not fixing a defect
in our software. That is fabricating the evidence Castle exists to check,
to get past Framer's server-side browser requirement. The goal's own hard
rules forbid it: no inventing browser signals and no bypassing Castle's
checks. Replaying captured Chrome answers from a cache is still inventing
them in the request where they matter, because no Chrome made them then.

I did not build the replay blob, did not run the SDK's probe in the saved
Chrome to harvest its answers, and closed the capture tab without storing
any canvas fingerprint cache.

## The legitimate paths that remain

RAM can still be saved without lying to Framer:

1. Keep one small real browser, shared. A single headless Chrome instance
   (the saved profile already runs one) can mint tokens for every Framerize
   worker on the machine. That replaces the current per-request browser
   pool (the 6-9 GB) with one resident browser using a few hundred MB.
   Tokens are genuine because a real browser makes them. This is the only
   design that both saves RAM and produces tokens Framer accepts.

2. Keep the existing honest browserless worker for everything that does
   not need Framer to believe a browser is present, and accept that Framer
   generation itself stays on the real-browser path.

3. Ask Framer for a supported server-to-server integration. Castle's own
   docs describe Risk/Filter API calls made from a server with an API
   secret, which is the documented non-browser route. Framer would have to
   expose it. That is outreach, not code, and is outside what I can do
   unilaterally.

Path 1 is the one I can implement now. It is a smaller change than the
realm, uses the saved Chrome that already exists, and stays inside the
security boundary. The token comes from a real browser every time.
