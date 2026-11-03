# Framerize server and Remote plan

## Decision

Build the Node-native Framerize server first. Keep the existing connector and
move it into `framerize-remote/` as Framerize Remote. Do not rewrite its
working transport or turn it into a server.

The connector migration and its review have separate ownership.
They start from the last merged
connector, not from this server branch. They may rename and relocate the
existing package and its CI while preserving its transport behavior. They do
not own server modules or live credentials.

Framerize is the overarching program. It owns accounts, credentials, sessions,
inference, streaming, model behavior, and the remote API. Framerize Remote is a
small remote connector to be built after the server contract is stable. Remote
must not contain account, inference, session, or minting code.

## Reusable evidence

The recovered `browserless` branch contains historical Node runtime experiments.
Use its tested resource-management patterns where they apply. Do not reproduce
anti-bot token minting, device impersonation, or credential-relay behavior.
The new runtime calls an inference API that the operator is authorized to use.

The existing Framer Next connector provides useful transport patterns: explicit
configuration, allowlisted headers, private credential references, redacted
logs, SSE passthrough, backpressure, cancellation, idle timeouts, shutdown,
JSON account metadata, package smoke tests, and native executable builds.
Keep those patterns in the server where they belong. Do not make Remote a second
server.

## Server-first architecture

The server has a small public API and separate internal modules:

1. Configuration validates endpoints, limits, and private file references.
2. Account directory reads JSON metadata and never exposes credentials.
3. Session provider owns private credentials and refresh coordination.
4. Upstream provider owns authorized API access and bounded request resources.
5. Inference service owns upstream calls, response conversion, and account
   status policy.
6. HTTP application exposes local and remote-compatible endpoints.
7. Lifecycle coordinates startup, request cancellation, and shutdown.

Each module has a narrow interface and fixture-based tests. Asynchronous work
accepts cancellation. Logs carry request IDs and outcomes, never credentials
or request bodies. No system CTR calls or implicit browser fallback are used.

## Future Remote boundary

Remote runs on a client computer and forwards API calls to one configured
Framerize URL. It exposes the same supported client API as the remote server.
It does not store accounts, select models, run inference, or launch a fallback
server. The server remains the only owner of account and credential behavior.

Reserve these behaviors for Remote when the server API is ready:

- `POST /v1/responses` and `POST /v1/chat/completions` forward unchanged.
- `GET /health` reports local process health without calling the server.
- `GET /ready` checks remote reachability and API compatibility without
  performing inference or account discovery.
- Client and server credentials remain separate and are never forwarded across
  the wrong boundary.
- Response bytes, status codes, stream events, and terminal markers pass through
  without local conversion.
- Disconnects cancel the corresponding remote request. Midstream failures never
  produce a synthetic successful terminator.
- Remote authentication failures do not trigger account rotation or fallback
  servers.

The server should expose a versioned readiness document with its API version
and supported routes. That document is the only compatibility dependency Remote
needs. It must not expose account metadata or credentials.

## Implementation order

1. Inventory the historical runtime and record its reusable resource patterns.
2. Build the server skeleton, configuration, logging, lifecycle, and fixtures.
3. Add JSON account metadata and private session access behind interfaces.
4. Implement the authorized upstream provider as a bounded internal service.
5. Implement inference and both client API routes through one service.
6. Verify short and long streams, tools, cancellation, cleanup, and isolation.
7. Add npm packaging, native binaries, CI, and reviewed release artifacts.
8. Run authorized live acceptance against the new server.
9. Finish Remote against the resulting remote contract.

Each stage merges independently after focused tests and review. Keep the
legacy browser-based fallback available but separate from the Node-native
server. Do not claim live compatibility from fixture tests.

## Deferred Remote design notes

Keep these earlier connector proposals as options rather than requirements:

- Preserve response bytes and headers without decompressing or reserializing.
- Keep local and remote credentials separate, with redacted diagnostics.
- Use an upstream target lock, forwarded-header allowlists, and no automatic
  retries or redirects.
- Propagate backpressure and disconnects. Never synthesize successful terminal
  events after a failed stream.
- Test short and long streams, fragmented UTF-8, large binary bodies, slow
  consumers, idle timeouts, and repeated shutdown on supported systems.
- Test the packed npm tarball and standalone executables outside the checkout.

The existing connector already implements several of these behaviors. Its
package migration preserves them for this phase. Account ownership and
provider-specific request preparation must move to the server in a later,
separately reviewed change. The current package does not yet satisfy that
final runtime boundary.

## Remote contract version 1

`POST /v1/responses` and `POST /v1/chat/completions` accept their documented
JSON bodies. `GET /health` reports local process health without an upstream
call. `GET /ready` returns 200 when the server can accept inference, or 503
when configuration or the authorized upstream is unavailable. Unknown routes
return 404. Health and readiness expose no account metadata.

Readiness returns `{ "apiVersion": 1, "status": "ready", "capabilities":
{ "responses": true, "chatCompletions": true, "streaming": true } }`.
An unavailable readiness response keeps `apiVersion` and sets `status` to
`unavailable` with one stable `reason`: `configuration_unavailable`,
`credentials_unavailable`, or `upstream_unavailable`. Remote rejects a different
major API version before sending inference. Readiness performs no inference or
account discovery and requires the same server token as inference.

Authentication failures return 401 with
`{ "error": { "type": "authentication_error", "code": "unauthorized",
"message": "Authentication required" } }`. Errors never include credentials,
account details, upstream response bodies, or internal paths. `/health` is
unauthenticated and returns only process health and API version.

Remote clients use `Authorization: Bearer <server-token>`. Remote may require
its own local key, but must replace it with the server token. It never forwards
client cookies or API keys. Server credentials stay in private files or
environment references and never appear in readiness, logs, or errors.

Each disconnect cancels its upstream request. A slow client applies
backpressure. Idle and header timeouts are configurable within Node's timer
limits; streams have no arbitrary total-duration limit. Midstream upstream
failure closes the client response abnormally. Never append a synthetic
successful terminal event. Preserve response status, body bytes, streaming
events, and terminal markers.

## Completion and release gates

Run unit and fixture integration tests on each change. Build and smoke-test
Linux x64, macOS arm64, and Windows x64 npm and binary artifacts on pull
requests, not only on publishing tags. Install the exact packed tarball and
run extracted binaries outside the checkout. Record version, full source
commit, and SHA-256 hashes before owner-approved publication. Activation is a
separate owner-controlled operation.
