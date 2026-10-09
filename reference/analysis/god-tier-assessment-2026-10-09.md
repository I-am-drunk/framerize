# God-tier assessment: what a deep exploit would have to be — 2026-10-09

After ~100 live probes and full decompilation of the client transport layer,
this is the definitive map of where exploitation depth does and does not
exist, and the exact cost of entry for each remaining class.

## The structural law of this target

Exploitable depth is inversely proportional to free-account reachability.
Framer's routing makes this explicit:

- Free workspace → stateless LLM proxy only (`/ai/v3/chat|messages|responses`)
- Paid workspace → MCP integration layer, image generation (real server work)
- Employee → baseten/fireworks internal proxies

A stateless proxy has no memory to corrupt and no state machine to confuse.
Every surface with server-side execution (outbound connections, OAuth
callbacks, tool dispatch, file handling) is behind a gate. This is why two
months of fuzzing the free surface yields metering quirks, not code bugs.

## Verified reachable surface from free accounts (complete)

Routes that exist (anything else under /ai/ is 404):

| Route | Auth | Depth |
|---|---|---|
| /ai/v3/chat/ | cookie OR ai-JWT | byte-tunnel to OpenRouter |
| /ai/v3/messages/ | cookie OR ai-JWT | byte-tunnel |
| /ai/v3/anthropic/messages/ | cookie OR ai-JWT | byte-tunnel |
| /ai/v3/openai/responses/ | cookie OR ai-JWT | byte-tunnel |
| /ai/v3/responses/ | cookie OR ai-JWT | byte-tunnel (guardrail 404s) |
| /ai/v3/fetch/ | ai-JWT | SSRF guard properly built (all bypass classes tested) |
| /ai/v3/refund/ | ai-JWT | turnId binding holds, quota one-time |
| /ai/v3/usage/total/ | web-JWT required (NOT cookie) | read-only |
| /ai/v1/mcp/* | web-JWT + paid plan | GATED (403) |
| /ai/v3/openai/images/ | paid plan | GATED (403) |
| /ai/v3/baseten/ /ai/v3/fireworks/ | employee | GATED (403) |

## The three god-tier classes, ranked by realism

### 1. SSRF-to-metadata in the MCP layer — realistic, ~15 bytes, needs one paid workspace

`POST /ai/v1/projects/{id}/mcp-connections {provider, name, returnUrl}` makes
Framer's server initiate an outbound connection to the provider and later
redirect through returnUrl. Client validates https-only/no-creds — but client
validation is advisory. If the server does not re-validate:
`provider: "http://169.254.169.254/latest/meta-data/iam/security-credentials/"`
→ pod IAM role → AWS account access. Survives any credit rework (not about
credits). Untestable for free: 403 gate confirmed via raw curl + valid JWT.
Cost of entry: one $10 Basic workspace.

### 2. Request desync between Framer's proxy and OpenRouter's edge

Chat route is a proven zero-validation byte-tunnel. If the proxy and
OpenRouter's edge disagree on request boundaries (CL.TE/TE.CL/H2 smuggling),
an injected second request is processed with Framer's upstream credentials
against arbitrary internal paths. Patch-resistant because it lives in proxy
semantics. Not testable from a browser (fetch normalizes) — needs raw socket
control: python/httpx or curl --http2-prior-knowledge from a shell. Free.

### 3. Supply-chain on framerstatic.com chunks

Every Framer user's browser imports content-hashed chunks. Writing to one =
code execution in every session. Requires a cloud/build compromise far above
any app-layer bug. Not a starting point.

## What is NOT worth more fuzzing

- Memory corruption: no native parsing reachable from a browser; everything
  is JSON into Go (memory-safe) or passed through to OpenRouter.
- Metering/credits: already fully mapped; the stream:false chat gap is the
  one durable finding and it is config, not code.
- featureCategory / envelope fields: strict allowlists, fully probed.
- SSRF on /ai/v3/fetch/: guard is correct (validates each redirect hop).

## Bottom line

No code-execution bug is reachable from a free workspace — that is the
target's design, not a testing gap. The realistic deep exploit is SSRF in
the MCP provider layer behind the paid gate. The only free, untested
deep class is proxy↔edge request desync, which needs raw-socket tooling.

## Desync battery results (2026-10-09, raw socket, HTTP/1.1 + HTTP/2)

Every classic smuggling primitive was rejected at the edge before reaching
the app — single strict front-end, no front/back disagreement:

| Probe | Result |
|---|---|
| CL.TE (CL + TE chunked) | 401 (one body interpretation, no smuggle) |
| TE.CL (TE + CL) | 401 (same) |
| dup content-length (differing) | 400 rejected |
| content-length with tab / plus-sign | 400 rejected (strict parser) |
| transfer-encoding: xchunked / identity,chunked | 501 rejected |
| HTTP/2 (ALPN h2, HEADERS+DATA) | 200, no downgrade path |

No request-boundary confusion exists between the edge and the app. The
gateway terminates and re-originates requests itself (it is the proxy), so
there is no second hop to disagree with on the way to OpenRouter.

This closes the last free deep class. Combined with the app-layer findings,
the honest conclusion stands: the only deep exploit available is SSRF in the
MCP provider layer, and it is behind a paid workspace gate.
