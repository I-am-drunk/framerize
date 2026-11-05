# Server

Server is the `framerize-server` package: a Node.js HTTP inference API with no
runtime dependencies. It holds the provider accounts and the Castle runtime.
Requires Node.js 22.12 or newer. The npm package is private and has no publish script.

The HTTP listener is the shared TypeScript Core, bundled as `dist/core.js`.
The installed package has no private workspace dependency. The upstream HTTP
provider exports `ProviderOptions`, `ProviderRequest` and `Provider` through
`framerize-server/provider`, with the required Core types included. The npm
archive contains generated runtime modules and declarations, not the source
tree. A small Node launcher remains in `bin/`. The account backend uses the
packaged Castle runtime and its TypeScript encoder.
Live provider compatibility is unverified.

## Run

From the repository root, with an accounts file as described in
[account configuration](docs/framer-backend.md#select-a-json-account):

```sh
npm ci --ignore-scripts
npm run build
FRAMERIZE_SERVER_BACKEND=framer \
FRAMERIZE_SERVER_ACCOUNTS=/srv/framerize/accounts.json \
node framerize-server/bin/framerize-server.js serve
```

The server listens on `127.0.0.1:8789`. Without an API token, clients need no
`Authorization` header. Set `FRAMERIZE_SERVER_TOKEN` to require one.
Every option has a `FRAMERIZE_SERVER_*` variable, and command-line options
override them. See the [environment guide](docs/environment.md).
Replace `serve` with `check` to validate configuration without contacting the provider.
Neither command loads `.env` files.

## Set up a background service

`up` saves the settings, installs a user service, waits for `/ready` and
prints a pairing link with a terminal QR code:

```sh
FRAMERIZE_SERVER_BACKEND=framer \
FRAMERIZE_SERVER_ACCOUNTS=/srv/framerize/accounts.json \
node framerize-server/bin/framerize-server.js up --lan
```

On first setup, choose one access mode. In a terminal, `up` asks.

- `--lan` listens on `0.0.0.0` and accepts only private addresses, as listed
  in the [environment guide](docs/environment.md#validation).
- `--tailscale` adds a Tailscale Serve HTTPS route to the local port.
  `--tailscale-serve-port` picks a port other than 443.
- `--no-lan` keeps the listener on this machine only.
- `--yes` accepts LAN access without asking.

The link is `<origin>/pair`, plus `#token=...` when Server has an API token.
`pair` prints it again. Pass `--json` to either command for a machine result.
`pair` never creates a Tailscale route. With saved Tailscale access, it
advertises the route only if it still points at this Server. Otherwise it
asks you to run `up --tailscale` first.
On the client machine, `frm LINK` pairs Remote with it. See the
[Remote guide](../framerize-remote/README.md#first-run).

Rerun `up` after changing settings. It restarts the service only when they
change. It stops its own previous service, then checks that the port is free
before starting the new one. Setup succeeds only when `/ready` answers from
the process the service manager reports as running, on both the local address
and the Tailscale route. A saved route follows a new local port or
`--tailscale-serve-port`. `--no-tailscale` removes the route `up` added.

If setup fails or receives SIGINT or SIGTERM, `up` tries to restore the
previous service, settings and Tailscale route. Cancellation stops pending
readiness checks and waits for the active service command to close before
rollback. On Linux and macOS, cancellation also stops descendants in the
command's process group, even if the command exits first. The group receives
SIGTERM, then SIGKILL after one second. Descendants that start a separate
process group can escape this cleanup. A cleanup error reports that command
processes may still be running.
A pending hostname lookup does not delay rollback.
A second signal can interrupt rollback before restoration is complete.
When a restore step also fails, the error reports both failures, and the
earlier state may be only partly restored.

### Service limits

- Linux uses the systemd user unit `framerize-server.service`; macOS uses the
  launch agent `dev.framerize.server`. Other platforms must use `serve`.
- On Linux, `up` runs `loginctl enable-linger` for the current user if
  lingering is off, so the service survives logout. It stops if that fails.
- The service restarts 5 seconds after an exit. Output goes to
  `FRAMERIZE_SERVER_HOME/server.log`.
- One service per user. `up` refuses a unit that serves another
  `FRAMERIZE_SERVER_HOME`.
- Tailscale must already be connected, with MagicDNS and HTTPS certificates
  on. `up` refuses a port that another route already serves, and never runs
  `tailscale up` or changes Funnel.
- The http backend needs `--upstream` or `FRAMERIZE_SERVER_UPSTREAM`; in a
  terminal, `up` asks for it. Port `0` is rejected.

`status` reports whether the service is installed and running; add `--json`
for unit path, log path, process ID and match details. `service start`,
`stop`, `restart` and `uninstall` manage it. Uninstall first removes the
Tailscale route that `up` added and clears the saved Tailscale access. It
keeps the rest of `server.json`. If another application has replaced that
route, uninstall leaves the route and the service in place and reports failure.
Removing a saved route needs Tailscale, because Server first checks that the
route is still its own. If Tailscale cannot report its status or routes,
uninstall and `up --no-tailscale` refuse and change nothing. Reconnect
Tailscale, then retry.

If Tailscale was deliberately removed and cannot be restored, use the same
`FRAMERIZE_SERVER_HOME` and run these commands from the repository root:

1. Run `node framerize-server/bin/framerize-server.js service stop`.
2. Back up `server.json` in `FRAMERIZE_SERVER_HOME`, which defaults to
   `~/.framerize-server`. Keep the file and its backup private with mode `0600`.
3. Remove only the top-level `access` property from the JSON. Preserve
   `version`, `config` and `environment`, and save valid JSON with mode `0600`.
4. Run `node framerize-server/bin/framerize-server.js service uninstall`.

This forgets Server's saved route without deleting Tailscale's own Serve
configuration. If you restore Tailscale later, remove the old Serve route
before reusing its backend port.

If the command reports "Invalid command or option", run `framerize-server --help`.
This covers unknown commands, unsupported options and missing option values.
The diagnostic does not print the supplied arguments.

If startup reports "Server configuration is invalid", check option values and
the [supported environment settings](docs/environment.md). For example, `--port`
must be an integer from 0 to 65535. With `--backend framer`, also check the
session in your [account JSON](docs/framer-backend.md#select-a-json-account).
An inline session must be valid. A session variable must be set and valid, and
a session file must be readable and valid. The variable can have any name.
Run `framerize-server --help` for supported options. The diagnostic links to the
environment guide. It does not print configuration values.

If startup reports "Account metadata file is unavailable or invalid", check
`--accounts-file`, read permissions, and the
[JSON account format](docs/framer-backend.md#select-a-json-account).
The diagnostic does not print the path or file contents.

If startup reports "Select one configured Framer account", check `--account-id`
or `FRAMERIZE_SERVER_ACCOUNT`, and `--accounts-file` or
`FRAMERIZE_SERVER_ACCOUNTS`. Use the exact `id` of one enabled account with a
`projectId` and a session. Malformed IDs also use this diagnostic for
the Framer backend. Omit the selector only when exactly one account is enabled.

If startup reports "Server could not listen on the configured host and port",
check `--host` or `FRAMERIZE_SERVER_HOST` and `--port` or
`FRAMERIZE_SERVER_PORT`. Use an available local address and port, then retry.
This diagnostic covers listener startup failures without exposing the address
or underlying error. It does not identify a specific operating-system error.

For the generic HTTP backend, name the upstream and its credential variable:

```sh
npm --prefix framerize-server start -- --upstream https://inference.example.test --authorization-env INFERENCE_API_KEY
```

Installed packages already contain the listener bundle, compiled configuration
and account loader. They do not need TypeScript or a build step. The package
includes Castle as `dist/castle.js`. TypeScript source stays out of the npm archive.
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

`--server-token-file` reads the API token from a regular file of at most
16 KiB. On Unix, group and other permission bits must be unset. The token is read
once at startup. Without a configured token, `/ready` and inference are open.

SIGINT or SIGTERM during startup cancels pending provider creation. The CLI
closes any provider returned afterward and does not open the listener. During
shutdown, signal handlers remain installed until cleanup finishes. Cleanup
failures still exit nonzero.

## HTTP contract

`GET /health` returns `{ "status": "ok", "apiVersion": 1 }` without authentication
or an upstream request. When an API token is configured, `GET /ready`,
`POST /v1/responses` and `POST /v1/chat/completions` require
`Authorization: Bearer <token>`. Without one, they accept every request.

Readiness returns API version 1, `status: "ready"`, and capabilities
`responses`, `chatCompletions`, and `streaming`, all true. Unavailability returns
503 with `status: "unavailable"` and one reason: `configuration_unavailable`,
`credentials_unavailable`, or `upstream_unavailable`. Readiness performs no
inference or account discovery. No endpoint exposes account metadata.
Every `/ready` response from `createServer` carries `x-framerize-server-pid`
with the server's process ID, including 401 replies to unauthenticated
requests. `up` uses it to confirm which process answered.

With a token configured, authentication failures return 401 and
`{"error":{"type":"authentication_error","code":"unauthorized","message":"Authentication required"}}`.
Other application errors have the same shape with `type: "invalid_request_error"`.

Unknown routes return 404 `not_found`, wrong methods return 405
`method_not_allowed`, invalid JSON objects return 400 `invalid_json`, and bodies
over `maxBodyBytes` return 413 `body_too_large`. Requests retain their original
UTF-8 JSON bytes after validation. The generic HTTP provider returns every
upstream response with its original status and body bytes. It keeps only the
`content-type`, `content-encoding`, `cache-control`, `x-request-id`,
`retry-after` and `x-should-retry` headers. A non-2xx body may use at most
65536 bytes on the wire, counted before any decompression. A chunk past that
limit ends the body with an error and closes the upstream exchange. A stalled
or truncated error body fails the same way, after the idle limit or when the
connection drops. The server never substitutes a generic body.

## Experimental custom routes

The Framer backend also accepts `POST /openai/v1/responses` and
`POST /anthropic/v1/messages`, with the same token rule as the bare routes.
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

`loadConfig(values, env)` validates explicit options and reads the API token
from `serverTokenEnv`, defaulting to `FRAMERIZE_SERVER_TOKEN`, or from
`serverTokenFile`. An injected `serverToken` is supported for embedding and
fixtures and is non-enumerable. Without a token the API is open. Upstream
credentials use `authorizationEnv` or `authorizationFile`, never both.

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

`accountsFile` optionally names a JSON array of accounts with `id`, `label`,
`enabled`, `model`, and `models`. It also accepts a `projectId` and a `session`.
The session is an inline string, or an object with exactly one `env` or `file`
field. Unknown fields, duplicate IDs, and malformed
records fail startup. The file is limited to 1 MiB. `loadAccounts(file)` returns
immutable records. The generic provider does not use these records. The
account backend requires an explicit selection when more than one account is
enabled. It never rotates accounts automatically.
`examples/metadata.example.json` contains no credentials. Keep actual account records outside Git.

The CLI logs JSON lines to stderr: `config.valid`, `server.listening` with its
port, and `request.finished` with the request ID, canonical route, status,
outcome and duration. These events carry no request bodies, account records or
credentials. The logger does not redact. A field passed to it, including an
error's name and message, is written as given. It keeps the first 2000
characters of each string and 100 entries per object or array, stops five
levels deep and marks circular references.

## Verify locally

With the root development dependencies installed, run from the repository root:

```sh
npm run build:server
npm --prefix framerize-server run check
npm --prefix framerize-server test
npm --prefix framerize-server run acceptance
npm --prefix framerize-server run pack:check
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

Run from the repository root:

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm ci --prefix framerize-server --ignore-scripts --no-audit --no-fund
npm run build:server
(cd framerize-server && npm pack --ignore-scripts)
npm --prefix framerize-server run pack:check -- --downloaded
npm --prefix framerize-server run binary
npm --prefix framerize-server run binary:package
npm --prefix framerize-server run binary:check
(cd framerize-server && node scripts/release-metadata.js)
```

See the [native build guide](../docs/native-server-build.md) for archive inputs
and the distinction between local builds and hosted qualification.

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
Run `(cd framerize-server && node scripts/merge-release-manifests.js)` from the
repository root. Set `RELEASE_DIST` to the absolute path containing the shared
tarball and the `linux-x64`, `macos-arm64`, `windows-x64` artifact directories.

Installed launcher comparison accounts for npm's removal of a CR from its
shebang line. Every remaining launcher byte and every other file stays strict.

The dedicated workflow uploads CI evidence only. No publication job, release tag,
deployment, live inference or automatic activation is part of this package.
Fixture results qualify only the exact tested artifacts and supported targets.
