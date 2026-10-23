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

The reviewed [Framer Server API](https://www.framer.com/developers/server-api-quick-start)
documents project-scoped keys and project operations. Its reference and the
published Plugin API index did not establish an AI completion/token method
for this integration. [Framer MCP](https://www.framer.com/mcp/) states that AI
usage follows the connected agent's plan or credits. That does not establish
a replacement for the requested Framer-backed inference. This is a bounded
documentation finding, not proof that no other supported integration exists.

The saved worker checks also have a narrower meaning than token acceptance.
They reject the observed outer V3-to-V2 fallback, require exactly one V3
selection without a Castle error log, and validate bounded nonempty UTF-8
and private-pipe framing. They do not classify the SDK string internally.
An earlier missing-TextEncoder control returned a string through V2 fallback;
the corrected V3 probe excluded that fallback but retained canvas diagnostics.
No internal fallback/error token is established for the live 403.

Next: trace the pinned V3 return and internal error/recovery paths to determine
whether the apparent success path can return an error value without the
outer error log. Preserve actual account history and require a justified
correction before another bounded live attempt. The source trace is pending;
it requires neither token capture nor SDK execution. The JSDOM support limit
was already recorded on October 20 and was reconfirmed, not newly discovered.
