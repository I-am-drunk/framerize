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

The focused static trace is now complete. Escaping exceptions reject the
current token operation. The main return path does not contain a blanket
catch that converts a rejected operation into another complete token string.
Individual capability checks can instead resolve diagnostic values: the
missing SharedWorker and failed canvas-context paths both do so. Those values
enter fields in the ordinary serialization path without an outer error.
Our worker explicitly requires SharedWorker to be absent. This establishes
why outer success does not imply complete browser behavior; it still does
not classify the actual token or explain the remote decision.

A separate [native canvas fixture](../scripts/experiments/native_canvas/README.md)
now passes real drawing, bounded PNG round-trip and input-refusal checks.
Its first run found incorrect context-type caching in JSDOM; the retained
dependency patch corrects that behavior for the native 2D backend. Neither
running service uses this experiment. The subsequent synthetic SDK integration
now passes with two local PNGs, zero JSDOM errors and the unchanged V3/device/
private-pipe/cleanup checks. Its first integration hit the 128 MiB group cap;
the retained 64 KiB file-verification buffer corrects that allocation issue
without changing the hash inventory. The passing run peaks at 84398080 bytes,
with zero swap/memory events and verified process/group absence. One process
warning remains unclassified. Tokens were discarded without retention.
This qualifies a local component, not the complete browser environment or
Framer acceptance. SharedWorker and WebGL remain absent. Next: package the
qualified variant for the isolated instance while preserving real account
history and the protected browser, before another bounded live check.
The JSDOM support limit was already recorded on October 20 and was reconfirmed,
not new. The application worker and both services remain unchanged.
