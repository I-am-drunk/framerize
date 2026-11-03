# Framerize server

A dependency-free Node.js HTTP server for an operator-authorized inference API.
Requires Node.js 22.12 or newer. The npm package is private and has no publish script.

## Run

Set `FRAMERIZE_SERVER_TOKEN` in the existing private environment. Keep the upstream
credential in a separate environment variable or private file. Do not pass secret
values as command-line arguments.

```sh
npm start -- --upstream https://inference.example.test --authorization-env INFERENCE_API_KEY
```

The server binds to `127.0.0.1:8789` by default. Use `--host` and `--port` to change
the listener. A reverse proxy must provide TLS for remote clients. Run
`npm start -- --help` for every supported option. Run
`node bin/framerize-server.js check` to validate configuration without contacting an upstream.

`--server-token-file` reads the server token from a private regular file, at most
16 KiB. On Unix, group and other permission bits must be unset. The token is read
once at startup. A missing server token leaves protected routes inaccessible.

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

## Inject a provider

```js
import { createServer } from 'framerize-server';
import { loadConfig } from 'framerize-server/config';

const app = await createServer(loadConfig({ port: 8789 }), { provider, logger });
await app.listen();
await app.close();
```

The provider implements `forward({ path, body, headers, signal })` and returns
`{ status, headers, body }`. Both bodies are async iterables of Buffers. Request
headers contain only `accept`, `content-type`, and `openai-beta`. No caller
credentials, cookies, account selectors, or arbitrary forwarding headers cross
this boundary. The provider owns its private upstream credentials.

`ready({ signal })` returns `true` or `{ ready: false, reason }`. The built-in
provider probes authenticated `GET /v1/models` and disposes the response without
reading its body. `close()` is idempotent and cancels outstanding operations.
The server accepts an injected provider, and the CLI constructs the built-in
provider only when `--upstream` is configured. Missing providers yield unavailable
readiness and 503 inference responses.

Providers must observe `signal` during all pending work, including body iteration.
The server closes a failed stream abnormally, propagates downstream backpressure,
and cancels on disconnect. It never appends a successful terminal event. Closing
the app stops admission, aborts requests, releases sockets, and closes the provider.

## Configuration and metadata

`loadConfig(values, env)` validates explicit options and reads the server token
from `serverTokenEnv`, defaulting to `FRAMERIZE_SERVER_TOKEN`. An injected
`serverToken` is supported for embedding and fixtures and is non-enumerable.
`serverTokenFile` overrides the default environment reference. Upstream credentials
use `authorizationEnv` or `authorizationFile`, never both.

Timeouts must be positive integers no greater than 2147483647 milliseconds:

- `headersTimeoutMs`, default 15000, bounds provider response-header wait.
- `requestHeadersTimeoutMs`, default 15000, bounds incoming HTTP headers.
- `idleTimeoutMs`, default 60000, bounds body inactivity and stalled client writes.
- `shutdownTimeoutMs`, default 5000, bounds provider cleanup.

There is no total request or stream deadline. `requestTimeoutMs` is rejected.
`maxBodyBytes` defaults to 16777216. The server buffers at most one bounded request
body per request to validate JSON before invoking the provider. Response bodies
stream without whole-response buffering. A paused client must resume before its
idle limit; provider body reads pause while downstream writes are blocked.

`accountsFile` optionally references a JSON array of metadata with `id`, `label`,
`enabled`, `model`, and `models`. Unknown fields, duplicate IDs, and malformed
records fail startup. The file is limited to 1 MiB. `loadAccounts(file)` returns
immutable records. Metadata does not select or rotate credentials or accounts.
`examples/metadata.example.json` contains no credentials. Keep actual account records outside Git.

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
These checks do not establish live compatibility or cross-platform binary support.
