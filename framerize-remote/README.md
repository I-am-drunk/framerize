# Framerize Remote

A local Framerize instance. It exposes the Framerize client API on loopback and
forwards every inference request to one configured remote Framerize server. It
runs no inference itself and holds no accounts, Castle, or browser state.

Remote runs the shared `@framerize/core` listener. The remote server owns
accounts, credentials, provider integration, and Castle.

## Build this restart

This TypeScript restart is not published or qualified for live inference yet.
From the repository root:

```sh
npm ci --ignore-scripts
npm run build
npm pack --workspace=framerize-remote --ignore-scripts
npm install -g ./framerize-remote-0.2.0.tgz --ignore-scripts
```

The generated package bundles the core listener. It has no runtime
dependencies and needs no compiler or development checkout.
TypeScript consumers need Node type declarations in their own development tools.

## First run

```sh
framerize-remote setup \
  --remote https://your-server.example \
  --server-token-file /private/framerize/server-token \
  --local-token-file /private/framerize/local-token \
  --start
```

Use paths to credentials you already provisioned. After readiness succeeds:

```text
Profile saved.
Remote readiness passed.
Local API: http://127.0.0.1:8788/v1
Stop: Ctrl+C
```

Ctrl+C closes the instance and prints `Stopped.`. Start the saved profile from
any directory with `framerize-remote serve`. No service manager is required.
If readiness fails, Remote exits without opening a listener and keeps the profile.
Use `framerize-remote check` after fixing the server or credential references.

A client then sends OpenAI-style requests to `http://127.0.0.1:8788/v1` with the
local token; Remote replaces it with the server token and forwards to the remote.

## Configuration

`setup` writes `~/.framerize-remote/framerize-remote.json`. Set
`FRAMERIZE_REMOTE_HOME` to an absolute directory to choose another location.
An explicit `--config FILE` overrides both. Old working-directory profiles
remain usable with `--config /absolute/path/to/profile.json`. Existing profiles
are not moved automatically. The profile stores
only non-secret references: the remote URL, credential references, listen host
and port, and the two explicit exposure flags. Tokens are never written to it.

Saves are atomic and owner-only on POSIX. New file references become absolute
paths; relative references in existing profiles resolve beside the profile.
Omit `--start` to save without loading credentials or contacting the server.

- `--server-token-env NAME` / `--server-token-file FILE`: the remote server
  token. Choose one.
- `--local-token-env NAME` / `--local-token-file FILE`: the local client token
  that callers present to this instance. Choose one. It is never forwarded.
- `--host` / `--port`: listen address, default `127.0.0.1:8788`.
  Port `0` lets the operating system select a port, printed when serving starts.
- `--allow-non-loopback`: required to bind a non-loopback host.
- `--allow-insecure-remote`: required for a plain-HTTP remote that is not
  numeric loopback. TLS validation is never disabled.

Remote does not auto-load `.env` files and never prints a token. Diagnostics use
fixed messages and never echo raw arguments, remote URLs, bodies, or credentials.

## Commands

- `help`, `version`: no listener, no credential read.
- `check [--json] [--tailscale]`: reports authenticated readiness; exit 0 ready, 1 otherwise.
- `serve [--json] [--host ADDRESS] [--port NUMBER] [--allow-non-loopback]`: checks readiness before listening.
- `setup --start [--json]`: saves the profile and uses the same startup path.

## Temporary listen settings

Use `framerize-remote serve --port 8789` to change the local port for one run.
Serve options never rewrite the profile. `SIGINT` and `SIGTERM` shut it down.
To accept connections outside loopback for one run:

```sh
framerize-remote serve --host 0.0.0.0 --allow-non-loopback
```

A non-loopback `--host` requires consent on that same command, even if the
saved profile permits a different exposed host. Without `--host`, the saved
host and consent apply. Use `--host 127.0.0.1` to restrict an exposed profile
temporarily. Local bearer authentication always remains required.

The listener uses HTTP. Protect non-loopback traffic with a private network
or a TLS proxy. Remote does not configure routing, firewall rules or public TLS.
The exposure notice reports the bind policy, not internet reachability.
JSON startup adds `exposure: "non-loopback-http"` only for exposed listeners.

## Endpoints

`GET /health` (unauthenticated), `GET /ready`, `POST /v1/responses`, and
`POST /v1/chat/completions`, all served by the shared core listener. Response
bytes, stream events, and terminal markers pass through unchanged.

Every inference first checks the remote's authenticated readiness. Failed,
oversized, incompatible or unfinished readiness blocks inference. Readiness
has a 64 KiB body cap and a 20-second whole-exchange limit. Response headers
have a 16 KiB cap without silently discarding later header pairs.

Inference waits up to five minutes for response headers. Set
`--inference-timeout-ms` during `setup`, or `inferenceTimeoutMs` in the saved
profile, to change this limit. Values must be integer milliseconds between
1 and 2147463647. Existing profiles use 300000. Configure the server's limit
separately; Remote cannot extend an upstream service's deadline.

Readiness still has a 15-second header wait and a 20-second overall limit.
Connection setup stays bounded at 15 seconds. Upload and response-body
inactivity stay bounded at 60 seconds. The inference limit includes connection
setup and request upload, so an active upload can also exhaust that limit.
After upload, silent computation uses the remaining inference time.
The local listener allows the readiness check plus the full inference limit.
An expired inference wait returns 504 without retrying the request.
Inference streams have no total lifetime limit after headers arrive.
Client cancellation closes the remote connection. Each exchange owns its
HTTP agent, ignores environment proxies and requires valid HTTPS certificates.

Startup output is compact by default. JSON mode covers setup, readiness,
listening, shutdown and command failures. Failed checks identify missing
profiles, the credential role, DNS/TCP/TLS failures, deadlines, HTTP refusals,
or invalid readiness data. Each diagnostic includes one suggested action.
JSON keeps `ready` and `reason`, with an added `diagnostic` on remote failures.

`framerize-remote check --tailscale` also runs local `tailscale status --json`.
It reports a fixed status label without machine names, IP addresses or login
links. It never logs in or changes Tailscale Serve, Funnel or other settings.
The subprocess has a 1.5-second deadline and a combined 256 KiB output limit.
Cancellation and limit failures terminate and reap it before returning.
Its status is advisory and does not change the readiness exit code, except
that cancellation exits nonzero. Local daemon status does not prove remote
reachability. Remote still needs an accessible server origin and valid token.

Browserless provider and Castle Library integration, native delivery and
release CI migration remain separate unfinished work.
