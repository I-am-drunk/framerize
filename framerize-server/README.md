# Framerize server

A Node.js HTTP server with no runtime dependencies for an operator-authorized inference API.
Requires Node.js 22.12 or newer. The npm package is private and has no publish script.

The HTTP listener is the shared TypeScript core, bundled as `dist/core.js`.
There is no private workspace dependency in the installed package. Existing
configuration validation, server-token loading and JSON account metadata compile
from TypeScript. The upstream HTTP provider also compiles from TypeScript and
exports `ProviderOptions`, `ProviderRequest` and `Provider` through
`framerize-server/provider`. Its declarations include the required core types
inside the package. CLI orchestration, logging and server composition also
compile from TypeScript. The npm archive contains generated runtime modules
and declarations, not the source tree. A small Node launcher remains in `bin/`.
The account backend uses the packaged Castle runtime and its TypeScript encoder.
This wiring does not establish live browserless compatibility.

## Run

Set `FRAMERIZE_SERVER_TOKEN` in the existing private environment. Keep the upstream
credential in a separate environment variable or private file. Do not pass secret
values as command-line arguments.

```sh
npm ci --ignore-scripts
npm run build
npm --prefix framerize-server start -- --upstream https://inference.example.test --authorization-env INFERENCE_API_KEY
```

Run these commands from the repository root. Installed packages already
contain the listener bundle, compiled configuration and account loader. They do not need
TypeScript or a build step. The package includes Castle as `dist/castle.js`.
TypeScript source stays out of the npm archive.
The main export includes `ServerProvider` and `ServerDependencies` types for
embedding. Providers can return synchronous or asynchronous responses; the
server normalizes them into the shared backend contract.

### Verify a native server with installed Remote

From the repository root, run:

```sh
npm run check:native -- /absolute/path/to/framerize-server /absolute/path/to/framerize-remote.tgz
```

The check copies the executable into a private folder with synthetic JSON account
metadata and an empty `PATH`. It installs Remote separately with npm offline and
tests authentication, streamed tool calls, readiness refusal and cancellation
against a loopback fixture. It checks process shutdown and removes its temporary files.
This verifies the supplied executable and package, not live provider compatibility.
Release builds still require a clean committed checkout and source-bound manifests.

The server binds to `127.0.0.1:8789` by default. Use `--host` and `--port` to change
the listener. A reverse proxy must provide TLS for remote clients. Run
`npm --prefix framerize-server start -- --help` for every supported option. Run
`node framerize-server/bin/framerize-server.js check` to validate configuration without contacting an upstream.

`--server-token-file` reads the server token from a private regular file, at most
16 KiB. On Unix, group and other permission bits must be unset. The token is read
once at startup. A missing server token leaves protected routes inaccessible.

SIGINT or SIGTERM during startup cancels pending provider creation. The CLI
closes any provider returned afterward and does not open the listener. During
shutdown, signal handlers remain installed until cleanup finishes. Cleanup
failures still exit nonzero.

## HTTP contract

`GET /health` returns `{ "status": "ok", "apiVersion": 1 }` without authentication
or an upstream request. `GET /ready`, `POST /v1/responses`, and
`POST /v1/chat/completions` require `Authorization: Bearer <server-token>`.

Readiness returns API version 1, `status: "ready"`, and capabilities
`responses`, `chatCompletions`, and `streaming`, all true. Unavailability returns
503 with `status: "unavailable"` and one reason: `configuration_unavailable`,
`credentials_unavailable`, or `upstream_unavailable`. Readiness performs no
inference or account discovery. No endpoint exposes account metadata.

Authentication failures return 401 and
`{"error":{"type":"authentication_error","code":"unauthorized","message":"Authentication required"}}`.
Other application errors have the same shape with `type: "invalid_request_error"`.

Unknown routes return 404 `not_found`, wrong methods return 405
`method_not_allowed`, invalid JSON objects return 400 `invalid_json`, and bodies
over `maxBodyBytes` return 413 `body_too_large`. Requests retain their original
UTF-8 JSON bytes after validation. Provider success responses retain their status,
body bytes and allowlisted headers, including `content-encoding`. The provider
converts non-2xx upstream bodies to a generic `upstream_http_error` response while
preserving status and safe headers. It never returns raw upstream error bodies.

## Experimental custom routes

The Framer backend also accepts `POST /openai/v1/responses` and
`POST /anthropic/v1/messages`. Both require the server bearer token.
Use `stream: false` or omit `stream` for completed JSON. Use `stream: true`
for client events. Background Responses jobs remain unsupported.

Both modes make one non-streaming provider request, to `/ai/v3/responses/`
or `/ai/v3/messages/` respectively. Client streaming never changes those paths
or enables provider streaming. There is no retry or route fallback.

The server collects the complete provider reply, up to 16 MiB, and checks its
JSON envelope before sending generated content. Valid tool calls, refusals and output-limit
results retain their original bytes. Broken, oversized or invalid replies return
a safe error with `x-should-retry: false`. Messages errors include `type: "error"`.
The inference deadline covers collection and client delivery, not just headers.
Parsing and buffering can use more memory than the reply's byte size.

For streaming clients, the server sends SSE headers after credential preparation
and comment keepalives during the provider wait. It sends protocol events once
the complete reply is validated, including tool arguments, usage and stop reasons.
This is not token-by-token generation. Incomplete results stay incomplete.
Provider failures after headers become sanitized stream errors, not HTTP errors;
disconnects and expired deadlines close the stream without a success terminator.
`x-should-retry: false` applies from the start. Remote forwards these bytes without
interpreting events. Neither keepalives nor slow clients extend the total deadline.
Responses events retain supplied item IDs and add a consistent ID where absent.
A missing token total is calculated from the supplied input and output counts.
Messages streams use zero for unavailable token counts when the provider omits usage.

An incomplete Responses result can contain an incomplete function call with
unfinished JSON in its `arguments` string. The server preserves that string,
the incomplete status and the limit reason. Clients must check the status
before parsing or executing a tool call. This does not permit truncated HTTP
replies or malformed outer JSON.

These routes have synthetic local coverage only. Current provider models,
options, version headers and live browserless acceptance remain unverified.
Readiness does not advertise custom-route support. The generic HTTP backend
does not implement these routes. Existing bare routes retain streaming behavior.

## Inject a provider

The server-only [session authentication module](docs/framer-auth.md) provides
cookie-to-token acquisition, caching and cancellation for account runtimes.
The [Framer backend](docs/framer-backend.md) uses it for Framer-specific
inference. `createAccountBackend` connects JSON account references to a private
auth instance and an application-supplied Castle minter. The CLI defaults to the
generic HTTP provider. `--backend framer` selects the account backend with the
packaged minter. See [account configuration](docs/framer-backend.md).

```js
import { createServer } from 'framerize-server';
import { loadConfig } from 'framerize-server/config';

const app = await createServer(loadConfig({ port: 8789 }), { provider, logger });
await app.listen();
await app.close();
```

The provider implements `forward({ path, body, headers, signal })` and returns
`{ status, headers, body }`. Request bodies are async iterables of Buffers;
response bodies may be synchronous or asynchronous iterables. Request
headers contain only `accept`, `content-type`, and `openai-beta`. No caller
credentials, cookies, account selectors, or arbitrary forwarding headers cross
this boundary. The provider owns its private upstream credentials.

`ready({ signal })` returns `true` or `{ ready: false, reason }`. The built-in
provider probes authenticated `GET /v1/models` and disposes the response without
reading its body. `close()` is idempotent and cancels outstanding operations.
The server accepts an injected provider, and the CLI constructs the built-in
provider when `--upstream` is configured or `--backend framer` is selected.
Missing providers yield unavailable
readiness and 503 inference responses.

Providers must observe `signal` during all pending work, including body iteration.
The server closes a failed stream abnormally, propagates downstream backpressure,
and cancels on disconnect. It never appends a successful terminal event. Closing
the app stops admission, aborts requests, releases sockets, and closes the provider.

A backend that fails or exceeds its shutdown deadline rejects `close()` with
`Backend shutdown failed`. The CLI still reports only its fixed fatal message.

## Configuration and metadata

`loadConfig(values, env)` validates explicit options and reads the server token
from `serverTokenEnv`, defaulting to `FRAMERIZE_SERVER_TOKEN`. An injected
`serverToken` is supported for embedding and fixtures and is non-enumerable.
`serverTokenFile` overrides the default environment reference. Upstream credentials
use `authorizationEnv` or `authorizationFile`, never both.

Timeouts must be positive integers no greater than 2147483647 milliseconds:

- `headersTimeoutMs`, default 15000, bounds readiness, credential preparation and connection setup.
- `inferenceTimeoutMs`, default 300000, bounds inference until response headers arrive, including provider preparation.
- `requestHeadersTimeoutMs`, default 15000, bounds incoming HTTP headers.
- `idleTimeoutMs`, default 60000, bounds body inactivity and stalled client writes.
- `shutdownTimeoutMs`, default 5000, bounds provider cleanup.

Set `--inference-timeout-ms` on the CLI, or `inferenceTimeoutMs` in the API.
This is separate from `headersTimeoutMs`, which still bounds credential and
connection setup and Framer request preparation. It does not bound the
inference response-header wait.
The five-minute default permits silent non-streaming computation without
extending readiness checks. It does not change an upstream service's limits.
Existing streaming routes have no total deadline after headers.
Custom completed-response routes retain the inference deadline through delivery.
`requestTimeoutMs` is rejected.
`maxBodyBytes` defaults to 16777216. The server buffers at most one bounded request
body per request to validate JSON before invoking the provider. Existing bare-route
responses stream without whole-response buffering. A paused client must resume before its
idle limit; provider body reads pause while downstream writes are blocked.

`accountsFile` optionally references a JSON array of metadata with `id`, `label`,
`enabled`, `model`, and `models`. It also accepts a `projectId` and a `session`
reference containing exactly one `env` or `file` field. Unknown fields, duplicate IDs, and malformed
records fail startup. The file is limited to 1 MiB. `loadAccounts(file)` returns
immutable records. The generic provider does not use these records. The
account backend requires an explicit selection when more than one account is
enabled. It never rotates accounts automatically.
`examples/metadata.example.json` contains no credentials. Keep actual account records outside Git.

An account may include `castleDeviceId` to preserve its existing Castle device
identity. The server passes it to every mint for that selected account. It must
be 1 to 256 characters using letters, digits, `.`, `_`, `~`, or `-`. Omitting it
keeps the library default. Do not invent a replacement ID during account import.
The value stays on the server, not in Remote configuration or client headers.
Preserving the ID does not establish live provider acceptance.

Logs contain request IDs, canonical routes, status, outcome, and duration. They
exclude request bodies, account records, credentials, and arbitrary error messages.

## Verify locally

```sh
npm run check
npm test
npm run acceptance
npm run pack:check
```

Tests use loopback fixtures. The package check installs the exact packed tarball
in a temporary directory and tests the installed CLI and HTTP application there.
Before importing the application, it rejects changed manifest metadata, runtime
dependencies, unexpected installed files, and changed source bytes. It permits
npm's leading `./` normalization in executable mappings and CR removal from the
launcher's shebang, but preserves all remaining bytes.
These checks do not establish live compatibility or cross-platform binary support.

## Native packaging

Build from a clean committed checkout on Linux x64, macOS arm64, or Windows x64.
Node 22.23.4 builds each CI executable. Node 22.12 and 24 run the source checks.
The pinned esbuild and postject development dependencies bundle the unchanged
CLI into a Node single executable. Only Node builtins may remain external.

```sh
npm ci
npm pack --ignore-scripts
npm run pack:check -- --downloaded
npm run binary
npm run binary:package
npm run binary:check
node scripts/release-metadata.js
```

The smoke check extracts the archive into a temporary path containing spaces,
clears the child environment and PATH, and runs help, configuration, health,
authentication, readiness and both inference routes against loopback fixtures.
It checks raw SSE and gzip bytes with separate synthetic client and upstream
credentials. All fixture processes, sockets and temporary files are closed.
The smoke also keeps health responsive during an active stream and verifies that
disconnecting its client closes the upstream request before server shutdown.
Unix checks graceful SIGTERM exit. Windows checks termination and cleanup,
not POSIX signal behavior. No installed Node is needed to run the executable.

Archives contain only the executable, project `LICENSE`, and the Node distribution
`NODE-LICENSE`, with license line endings normalized to LF. Packaging requires the
Node distribution license beside the Node executable or its parent directory.
`NODE_DISTRIBUTION_LICENSE` may explicitly point to that distribution license.

The final metadata step runs after archive creation. `dist/release.json` binds the
package name, version, full source commit, target and Node version to SHA-256 hashes
for the executable, archive, shared npm tarball and both licenses. `dist/SHA256SUMS`
contains those same five entries. The build record rejects stale executables.

CI builds the shared npm tarball once, tests that exact artifact on every target,
and compares installed files to committed bytes even on CRLF checkouts. It then
builds, extracts and tests each native archive. The manifest job verifies all three
targets, a common source/version/Node version, identical licenses and the shared
tarball hash. It writes six payloads plus aggregate metadata under `dist/qualified/`.
Run `node scripts/merge-release-manifests.js` with `RELEASE_DIST` pointing to the
shared tarball and the `linux-x64`, `macos-arm64`, `windows-x64` artifact directories.

Installed launcher comparison accounts for npm's removal of a CR from its
shebang line. Every remaining launcher byte and every other file stays strict.

The dedicated workflow uploads CI evidence only. No publication job, release tag,
deployment, live inference or automatic activation is part of this package.
Fixture results qualify only the exact tested artifacts and supported targets.
