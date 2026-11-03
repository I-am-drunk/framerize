# Framerize

## Browserless Node work in progress

The `browserless-node` branch contains the TypeScript restart. It is a development
snapshot, not a release or a deployed service. Framerize Remote replaces the
previous connector name throughout the active packages and commands.

Local tests cover the shared server, installed Remote package, authentication,
streaming and shutdown. The [Framer backend](framerize-server/docs/framer-backend.md)
now loads selected JSON account references and maps provider requests through
the actual listener in synthetic tests. Castle Library now collects and encodes
without an external encoder. The server CLI selects a JSON account with
`--backend framer --accounts-file FILE --account-id NAME`. Its package includes
Castle Library, and the native build embeds that runtime in the executable.
Live browserless inference, current hosted release qualification and npm
publication are not complete. Do not use old release evidence to approve this
snapshot.

TypeScript packages for a browserless Node server and a small local connector.
The connector works offline against synthetic providers. Live browserless
provider compatibility is not yet established.
Use Node.js 22.12 or newer. Run source commands from this repository's root.

## Choose a package

- [Framerize server](framerize-server/README.md) exposes authenticated Responses
  and Chat Completions routes. Its default listener is `127.0.0.1:8789`.
- [Framerize Remote](framerize-remote/README.md) forwards local client requests to
  a configured endpoint. Its default listener is `127.0.0.1:8788`.
- [Castle Library](framerize-castle/README.md) contains the server-side minting
  API and assets. Only the server includes it, never Remote or the shared core.
- [Shared core](docs/typescript-layout.md) supplies the HTTP listener and
  readiness contract used by both server and Remote.

Remote is not published to npm yet. Use the source commands here instead of
assuming that `npx framerize-remote` installs this revision.

## Build and inspect

Build before running either source launcher. Published packages will contain
compiled JavaScript and declarations, not require TypeScript at runtime.

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm run build
node framerize-server/bin/framerize-server.js --help
node framerize-remote/bin/framerize-remote.js --help
node framerize-server/bin/framerize-server.js check
node framerize-remote/bin/framerize-remote.js check
```

Server `check` validates local configuration without contacting its upstream.
Remote `check` contacts the configured server's authenticated readiness endpoint.
Neither check proves live inference compatibility.

## Configure an endpoint

### Remote

Provision two separate credentials: the token accepted by your server and
the local token your clients will present to Remote. Pass file references,
never token values in command arguments:

```sh
node framerize-remote/bin/framerize-remote.js setup \
  --remote https://your-server.example \
  --server-token-file /private/framerize/server-token \
  --local-token-file /private/framerize/local-token \
  --start
```

Use the server origin without `/v1`. Setup saves credential references in
`~/.framerize-remote/framerize-remote.json`, checks readiness, and starts the
local API at `http://127.0.0.1:8788/v1`. It does not provision a remote server.
Later, use `serve` from any directory or `check --tailscale` to diagnose the
connection. Neither CLI automatically loads `.env` files. See the
[Remote guide](framerize-remote/README.md) for compact output, JSON mode and
explicit non-loopback access.

### Server

The server reads `FRAMERIZE_SERVER_HOST`, `FRAMERIZE_SERVER_PORT`, and
`FRAMERIZE_SERVER_UPSTREAM`. Explicit CLI flags override environment values;
unset variables use the defaults. An empty upstream selects no provider.

A POSIX-shell example validates these settings without starting a listener or
contacting the upstream:

```sh
FRAMERIZE_SERVER_HOST=127.0.0.1 \
FRAMERIZE_SERVER_PORT=8789 \
FRAMERIZE_SERVER_UPSTREAM=http://127.0.0.1:9000 \
  node framerize-server/bin/framerize-server.js check
```

This does not start the example upstream or establish readiness. Keep secrets
out of `FRAMERIZE_SERVER_UPSTREAM`; use the credential-reference options below.
See [server environment configuration](framerize-server/docs/environment.md)
for validation and programmatic precedence.

It reads its client-facing token from `FRAMERIZE_SERVER_TOKEN` by default and
uses a separate upstream credential reference. Follow the
[server setup instructions](framerize-server/README.md#run) for those options.

## Current integration status

Both products use the same typed listener. Remote replaces the local credential
with the server token and forwards opaque response bytes. Account selection,
provider requests and Castle Library belong on the server, never in Remote.
Local installed-pair tests cover streaming, tool calls, cancellation and auth.
They use a synthetic upstream, not a live inference provider.

The [package workflow](docs/package-coexistence-ci.md) and
[release candidate checks](docs/package-release.md) require new evidence for
changed packages. Historical JavaScript and native reports do not qualify this
TypeScript restart. Hosted platform checks, live browserless compatibility
and npm publication remain open. The [publication workflow](docs/remote-publication.md)
requires reviewed live evidence and environment approval before publishing
the original Remote archive.
The [native server build](docs/native-server-build.md) now binds each executable
to the verified npm archive rather than bundling loose checkout files.

For development checks, use each package's README. The
[two-package check](docs/package-coexistence.md) verifies installed package
coexistence. Fixture and installation results do not establish live inference
compatibility or publication of a release.
