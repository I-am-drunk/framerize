# Castle runtime support

Reviewed the public documentation on 2026-10-23 alongside the current worker
source and the saved isolated request result. This was documentation and
source inspection only; no SDK, build, service check or request was run.

The [Browser SDK documentation](https://docs.castle.io/docs/sdk-browser)
requires a browser with native Promise support. Its 3.0 migration section
says: "Pointing the SDK at a mocked DOM such as JSDOM is no longer supported.
No replacement." It describes browser workers, styles and behavioral signals.
Tokens should be freshly generated for each request, expire after 120 seconds,
and travel in the request body.

The [integration guide](https://docs.castle.io/docs/integration-guide)
separates frontend token generation from backend Risk/Filter API calls.
The [server SDKs](https://docs.castle.io/docs/sdk-download), including Node,
consume the frontend token; they are not a documented replacement token
generator. [Mobile SDKs](https://docs.castle.io/docs/sdk-mobile) cover Android,
iOS, Flutter and React Native, not a generic Linux server runtime.
[Mock tokens](https://docs.castle.io/docs/test-tokens) are sandbox-only.

Our worker uses JSDOM, blocks resource loading, and deliberately keeps the
truthful Framerize user agent. Its local checks establish token production,
bounded lifetime and cleanup, not acceptance of the device signals by Framer.
The native transport already serializes castleToken into the JSON body.
The current worker is outside the documented Browser SDK runtime support.
That support mismatch does not independently identify the reason for a 403.

The saved 23:27 UTC isolated attempt sent once and received a complete Framer
security-error JSON response after 2.63 seconds. Cleanup was confirmed and
the browser service was unchanged. Adding Origin and Referer was insufficient
for that attempt. No authenticated text or live interruption was accepted.

Next: establish whether Framer documents an accepted native authentication
and token source before choosing another compatibility change. Keep the
existing experiment and evidence, preserve actual account history, and require
a justified correction before another bounded live attempt. The supported
runtime question is unresolved for this Linux worker; it is not a build issue.
