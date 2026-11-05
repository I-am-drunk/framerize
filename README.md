# Framerize

Framerize is a Node.js inference API server. Framerize Remote gives a client
machine a local endpoint that forwards requests to that server.

The server owns provider accounts and Castle. Remote holds neither.

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

## 2. Start Server

Write an accounts file, for example `/srv/framerize/accounts.json`, with a
real session in place of `session=...`:

```json
[{ "id": "personal", "projectId": "your-project-id", "session": "session=..." }]
```

The file holds a live credential. Keep it out of Git. The
[account guide](framerize-server/docs/framer-backend.md#select-a-json-account)
also covers sessions read from an environment variable or file.

```sh
FRAMERIZE_SERVER_BACKEND=framer \
FRAMERIZE_SERVER_ACCOUNTS=/srv/framerize/accounts.json \
FRAMERIZE_SERVER_HOST=0.0.0.0 \
node framerize-server/bin/framerize-server.js serve
```

Without `FRAMERIZE_SERVER_HOST`, Server listens on loopback only. With no API
token, any client that reaches the port can run inference on the account.
Expose it only on a network your friends alone can reach, such as a tailnet.
Set `FRAMERIZE_SERVER_TOKEN` to require a bearer token. Replace `serve` with
`check` to validate configuration without contacting the provider. Every option
has a variable; see the [environment guide](framerize-server/docs/environment.md).

## 3. Connect Remote on the client machine

Remote setup still needs a server token file and a separate local token file.
If Server runs without a token, any valid token value works in the first file.
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
