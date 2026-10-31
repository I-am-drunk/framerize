# Step 1 result: what the Castle SDK actually asks the browser

Method: ran the production module chain (wrapper, timeout, logger, services,
bundler, pinned `castleSDKV3-LNGA36I3.mjs`, hash verified against pins.mjs)
inside the existing JSDOM worker, with every `window` getter and event
registration wrapped in a recording proxy. Networking was never enabled.
The mint still succeeded and produced a 1792-character token, so the trace
captured a complete, real collection pass.

Numbers: 299 recorded accesses, 126 unique API paths, 21 console errors.

## The three groups of questions

1. Property reads. 40 navigator fields and 10 screen fields, read once or
   twice each. These are cheap to answer: userAgent, platform, languages,
   hardwareConcurrency, deviceMemory, maxTouchPoints, cookieEnabled, vendor,
   productSub, userAgentData, webdriver, plus screen width/height/availWidth/
   availHeight/colorDepth and devicePixelRatio.
2. Storage and randomness. localStorage get/set/removeItem, sessionStorage,
   crypto.getRandomValues, crypto.subtle, performance.now and timeOrigin.
3. Behavior hooks. About 60 addEventListener registrations (mouse, keyboard,
   touch, clipboard, visibility, focus, scroll, wheel, resize, online). These
   stay silent unless events fire. A headless run simply reports no activity.

Full inventory: `trace-output.json` next to this file.

## The loud failure

19 of the 21 console errors are the same JSDOM line:
`Not implemented: HTMLCanvasElement.prototype.getContext (without installing
the canvas npm package)`.

That is the SDK attempting canvas fingerprinting 19 times and getting an
exception every time. Real Chrome never throws there. A server comparing
token answers against a claimed desktop-Chrome user agent sees canvas data
missing where canvas data must exist. This is a behavioral difference, not
a name check: the string "jsdom" appears nowhere in the SDK source.

Also notable: `navigator.userAgentData` was read (2 gets). JSDOM leaves it
absent, which matches the worker's documented "userAgentData absent" state.
Real desktop Chrome has it. That is a second, quieter mismatch.

## What this changes

The rejection theory now has named, counted evidence: at least 19 canvas
probe failures plus an absent userAgentData, both inconsistent with the
user agent the token claims. Either alone can explain a 403 from a fraud
system. No TLS argument is needed yet.

## What step 2 must supply

A no-JSDOM realm needs: the 40 navigator fields, 10 screen fields, storage,
crypto, performance timing, the event-registration surface, and two things
JSDOM cannot give: a working canvas getContext whose drawing answers match
the user's real browser, and a userAgentData matching that browser. All
values copied from the user's Camoufox/Chrome profile. Nothing invented.

Trace artifacts: `trace_run.mjs`, `trace_support.mjs`, `trace-output.json`.
No live request, no token sent anywhere, no service touched.
