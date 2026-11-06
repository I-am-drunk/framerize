# Framerize

Framerize serves an inference API backed by Framer accounts. Install the
`framerize` package on the machine that holds the accounts, or the smaller
`framerize-remote` package on a client machine. Both provide the same command
names: `frm` and `framerize`.

| Package | Where | Default address |
| --- | --- | --- |
| [`framerize`](framerize-server/README.md) | Machine with the account | `127.0.0.1:8789` |
| [`framerize-remote`](framerize-remote/README.md) | Client machine | `127.0.0.1:8788` |

Server holds the accounts and Castle. Remote holds neither. Both use the
HTTP listener in [Core](framerize-core/README.md).

`main` is the development branch. Its npm packages are not published yet.
Build and install local npm archives as described below. Live provider
compatibility remains unverified.

## 1. Install the commands

Use Node.js 22.12 or newer. Build the development packages once from the
repository root, then copy the needed archive to each machine:

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm run build
npm pack ./framerize-server --ignore-scripts
npm pack --workspace=framerize-remote --ignore-scripts
```

On the Server machine:

```sh
npm install --global --ignore-scripts ./framerize-0.1.0.tgz
```

On each client machine:

```sh
npm install --global --ignore-scripts ./framerize-remote-0.2.0.tgz
```

Either package installs both `frm` and `framerize`. Choose one package per
global npm prefix. Before switching roles, run `frm service uninstall` if you
installed its service, then uninstall the npm package. Do not use `--force`.
npm creates the command links in its global prefix. On Linux and macOS, that prefix's
`bin` directory must be on `PATH`; on Windows, the prefix itself must be on
`PATH`. Run `npm prefix -g` to find it. Both commands work outside the checkout.

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
framerize up --lan
```

`up` saves the settings, starts Server as a background service on Linux or
macOS, and prints a pairing link and QR code. `--lan` accepts only private
addresses; `--tailscale` adds a Tailscale Serve HTTPS route instead.
To run in the foreground on any platform, enable the same LAN filter with:

```sh
FRAMERIZE_SERVER_BACKEND=framer \
FRAMERIZE_SERVER_ACCOUNTS=/srv/framerize/accounts.json \
FRAMERIZE_SERVER_LAN=true \
framerize serve
```

`--lan` and `--tailscale` belong to `up`; `serve` does not accept those flags.

With no API token, any client that reaches the port can run inference on the
account. Set `FRAMERIZE_SERVER_TOKEN` to require a bearer token; the pairing
link then carries it. The [Server guide](framerize-server/README.md#set-up-a-background-service)
covers service limits, and the [environment guide](framerize-server/docs/environment.md)
lists every variable.

## 3. Pair Remote on the client machine

For a background Server, pass the link that `up` printed to `frm`. Run
`framerize pair` on the Server machine to print it again.

```sh
frm 'http://192.168.1.20:8789/pair#token=...'
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
The [request performance checks](docs/request-performance.md) measure synthetic
server and Remote workloads and gate request and byte-count regressions in CI.

Contributor rules are in [AGENTS.md](AGENTS.md) and
[CONTRIBUTING.md](CONTRIBUTING.md). See the
[TypeScript layout](docs/typescript-layout.md) for package boundaries,
[package checks](docs/package-coexistence.md) for installed use and
[native build guide](docs/native-server-build.md) for executable qualification.
