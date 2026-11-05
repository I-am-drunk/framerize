# Framerize

Framerize serves an inference API backed by Framer accounts. Server runs on
the machine that holds the accounts. Remote, the `frm` command, runs on each
client machine and forwards requests to Server.

| Run this | Where | Default address |
| --- | --- | --- |
| [Server](framerize-server/README.md) | Machine with the account | `127.0.0.1:8789` |
| [Remote](framerize-remote/README.md) | Client machine | `127.0.0.1:8788` |

Server holds the accounts and Castle. Remote holds neither. Both use the
HTTP listener in [Core](framerize-core/README.md).

`main` is the development branch. No revision is published to npm, and
`npx framerize-remote` does not install this source. Live provider
compatibility is unverified. Use the source commands below.

## 1. Build the source

Use Node.js 22.12 or newer. On each machine, from the repository root:

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm run build
```

For this source build, run commands from the repository root:

- Use `node framerize-server/bin/framerize-server.js` wherever a guide says `framerize-server`.
- Use `node framerize-remote/bin/frm.js` wherever a guide says `frm`.

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
node framerize-server/bin/framerize-server.js up --lan
```

`up` saves the settings, starts Server as a background service on Linux or
macOS, and prints a pairing link and QR code. `--lan` accepts only private
addresses; `--tailscale` adds a Tailscale Serve HTTPS route instead.
To run in the foreground on any platform, enable the same LAN filter with:

```sh
FRAMERIZE_SERVER_BACKEND=framer \
FRAMERIZE_SERVER_ACCOUNTS=/srv/framerize/accounts.json \
FRAMERIZE_SERVER_LAN=true \
node framerize-server/bin/framerize-server.js serve
```

`--lan` and `--tailscale` belong to `up`; `serve` does not accept those flags.

With no API token, any client that reaches the port can run inference on the
account. Set `FRAMERIZE_SERVER_TOKEN` to require a bearer token; the pairing
link then carries it. The [Server guide](framerize-server/README.md#set-up-a-background-service)
covers service limits, and the [environment guide](framerize-server/docs/environment.md)
lists every variable.

## 3. Pair Remote on the client machine

For a background Server, pass the link that `up` printed to `frm`. Run
`framerize-server pair` on the Server machine to print it again.

```sh
node framerize-remote/bin/frm.js 'http://192.168.1.20:8789/pair#token=...'
```

`frm` saves the Server origin and any token from the link in
`~/.framerize-remote/framerize-remote.json`. It checks Server readiness, then
starts Remote as a background service. Clients use `http://127.0.0.1:8788/v1`.
In a terminal, `frm` then offers to install or update Claude Code and Codex.
Start one with `frm client claude` or `frm client codex`.

For a foreground Server, which prints no pairing link, run
`frm up http://SERVER:8789` with its private address. Set `FRM_SERVER_TOKEN`
if Server requires a token. Running `frm` with no arguments on a new machine
asks for the link. `frm serve` runs Remote in the foreground instead,
for platforms without a service manager. See the
[Remote guide](framerize-remote/README.md) for variables, clients and service limits.

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
