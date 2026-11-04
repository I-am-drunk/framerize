# Framerize

Framerize is a Node.js inference API server. Framerize Remote gives a client
machine a local endpoint that forwards requests to that server.

The server owns provider accounts, provider credentials and Castle. Remote
keeps its local client token separate from the token it uses to reach the server.

| Run this | Where | Default address |
| --- | --- | --- |
| [Framerize server](framerize-server/README.md) | Machine with provider access | `127.0.0.1:8789` |
| [Framerize Remote](framerize-remote/README.md) | Client machine | `127.0.0.1:8788` |

`main` is the current development branch. This source revision is not a
published release. Live provider compatibility and release qualification remain
unfinished. Use the source commands below; `npx framerize-remote` does not install
this revision.

## 1. Build the source

Use Node.js 22.12 or newer. Run these commands from the repository root on each
machine where you use a source launcher:

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm run build
```

## 2. Configure and start the server

Prepare an authorized account and its private session reference using the
[account configuration guide](framerize-server/docs/framer-backend.md#select-a-json-account).
Keep the server token in a separate private file. Pass file paths or environment
variable names to the CLI, never credential values.

```sh
node framerize-server/bin/framerize-server.js serve \
  --backend framer \
  --accounts-file /private/framerize/accounts.json \
  --account-id personal \
  --server-token-file /private/framerize/server-token
```

Replace `serve` with `check` to validate local configuration without contacting
the provider. This does not establish that an account can perform inference.
Use Ctrl+C to stop the server.

The default listener is local to the server machine. For clients on another
machine, use your TLS reverse proxy's HTTPS origin in the Remote command below.
The [server guide](framerize-server/README.md#run) covers the generic HTTP
backend, listener options and embedding. Neither CLI automatically loads `.env`.

## 3. Connect Remote on the client machine

Use the server's token file and a different private token file for local clients.
The server URL is its origin, without `/v1`:

```sh
node framerize-remote/bin/framerize-remote.js setup \
  --remote https://your-server.example \
  --server-token-file /private/framerize/server-token \
  --local-token-file /private/framerize/local-token \
  --start
```

Setup saves credential references in
`~/.framerize-remote/framerize-remote.json`, checks the server's readiness and
starts the local API. Configure your client with `http://127.0.0.1:8788/v1`
and the local token. Setup does not provision a server or import an account.

Use `serve` for later starts and `check --tailscale` for connection diagnostics.
The [Remote guide](framerize-remote/README.md) covers saved profiles, JSON output
and explicit non-loopback access. Both launchers support `--help`.

## Verification and development

Readiness and synthetic tests do not prove live provider compatibility.
[Current status](docs/wip-status.md) separates those checks from release gates.
The [single-request validator](docs/inference-validation.md) documents the
separate procedure for an authorized live check.

Contributor rules are in [AGENTS.md](AGENTS.md) and
[CONTRIBUTING.md](CONTRIBUTING.md). See the
[TypeScript layout](docs/typescript-layout.md) for package boundaries,
[package checks](docs/package-coexistence.md) for installed use and
[native build guide](docs/native-server-build.md) for executable qualification.
