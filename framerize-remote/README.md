# Remote

Remote is the `framerize-remote` package and its `frm` command. It serves the
Framerize client API on loopback and forwards every inference request to one
Server. It runs no inference and holds no accounts, Castle or browser state.

Remote runs the shared Core listener. Server owns the accounts, provider
integration and Castle. The older `framerize-remote` command remains available.

## Build from source

Remote is not published to npm. From the repository root:

```sh
npm ci --ignore-scripts
npm run build
npm pack --workspace=framerize-remote --ignore-scripts
npm install -g ./framerize-remote-0.2.0.tgz --ignore-scripts
```

The package bundles the Core listener. It has no runtime dependencies and
needs no compiler or development checkout.
TypeScript consumers need Node type declarations in their own development tools.

## First run

Copy the link that `framerize-server up` or `framerize-server pair` printed:

```sh
frm 'http://192.168.1.20:8789/pair#token=...'
```

`frm LINK` is the same as `frm up LINK`. It saves the Server origin and any
token from the link in `FRM_HOME/framerize-remote.json`, mode 0600. It checks
Server readiness, then installs and starts the background service. Clients use
`http://127.0.0.1:8788/v1`. A link without `#token=` is for a Server with no
API token. `frm up ORIGIN` saves a plain origin and keeps any saved token.

Running `frm` with no command and no saved profile does the same in a terminal:
it asks for a link or address. In a terminal, `up` then offers to set up
Claude Code or Codex. `frm up --json` prints the result and asks nothing.

`up` checks that the managed service has started its local API before reporting
success. Before startup completes, Ctrl+C cancels the pending check or service
command. A pending hostname lookup does not delay rollback.
If setup fails or is cancelled, `up` attempts to restore the previous
profile and service state. A previously stopped service stays stopped. If
restoration fails, the error includes the original failure and reports that
restoration is incomplete. If the profile cannot be restored, `up` does not restart
the service with changed settings.
Rerunning `up` with unchanged settings checks the running service without restarting
it. Where no service manager is available, `up` saves the profile and asks you to
keep `frm serve` running.

`FRM_REMOTE`, `FRM_HOST` and `FRM_PORT` apply to `up`. A saved port `0` becomes
8788, because the service needs a fixed port. `FRM_SERVER_TOKEN` and
`FRM_LOCAL_TOKEN` also apply: `up` saves their values in the profile before it
checks readiness, so the service can use them. A token in the link takes
precedence over `FRM_SERVER_TOKEN`.

## Run in the foreground

`frm serve` runs the saved profile until Ctrl+C. Without a profile, set
variables for one run; nothing is saved:

- `FRM_REMOTE`: Server origin, such as `http://100.64.0.1:8789`.
- `FRM_SERVER_TOKEN`: token sent to Server, if Server requires one.
- `FRM_LOCAL_TOKEN`: token local clients must send. Without it, local
  clients connect with no `Authorization` header.
- `FRM_HOST`, `FRM_PORT`: listen address, default `127.0.0.1:8788`.
- `FRM_HOME`: profile directory, default `~/.framerize-remote`.

If readiness fails, `serve` exits without opening a listener. Use `frm check`
after fixing the address or token.

Remote removes the local client's `Authorization` header before forwarding.
It sends the server token instead, when one is configured.

## Background service

`frm up` installs and starts the service. `frm service install` does the same
for an existing profile. The service runs `frm serve` with the profile in
`FRM_HOME`. It is a systemd user unit `frm.service` on Linux, or the launch
agent `dev.framerize.frm.service` on macOS.
Other platforms report the service as unavailable; use `frm serve` there.

- Install fails without a saved profile.
- The service never reads shell variables. A profile that names a token
  variable fails install. `frm up` saves token values instead, from a pairing
  link or from `FRM_SERVER_TOKEN` and `FRM_LOCAL_TOKEN`.
- One service per user. Install refuses a unit that serves another `FRM_HOME`,
  and `start`, `stop`, `restart` and `uninstall` leave that unit unchanged.
- systemd restarts it 5 seconds after an exit; launchd keeps it alive with a
  5-second throttle. Output goes to `FRM_HOME/frm-service.log`.
- On Linux, install first checks that the systemd user manager answers and
  that lingering is on, so the service survives logout and starts at boot.
  If lingering is off, it runs `loginctl enable-linger --no-ask-password` for
  your user and checks again. If that fails, install stops before writing a
  unit and prints the next step, usually `sudo loginctl enable-linger
  "$(id -un)"` followed by `frm up`. Do not run `frm` itself with sudo.
- `frm up` runs the same check when the service is already current and
  running, so it repairs lingering without restarting unchanged settings.

`frm service status` reports whether the unit is installed, running and matches
the current profile. It does not report lingering. Run `frm up` or
`frm service install` again to repair it.
Remote never configures Tailscale Serve or Funnel. Server's `up --tailscale`
publishes the route, and its pairing link then carries the HTTPS origin.

## Coding clients

`frm client claude` or `frm client codex` starts that client against the
running Remote. Arguments after `--` pass through unchanged, such as
`frm client codex -- --model NAME`.

- Claude Code gets `ANTHROPIC_BASE_URL=<Remote>/anthropic` and the local token
  as `ANTHROPIC_AUTH_TOKEN` for that process only. The child does not inherit
  `ANTHROPIC_API_KEY` or the `CLAUDE_CODE_USE_BEDROCK`, `_VERTEX`, `_FOUNDRY`
  and `_ANTHROPIC_AWS` flags; your shell keeps them. Before launch it reads the user,
  project, local and managed Claude settings. A different base URL in a user
  or project file is overridden with `--settings`. The launch stops and names
  the file if a settings file sets a different auth token, an API key, a cloud
  provider flag or `apiKeyHelper`, or if managed settings set a base URL or
  force a login method.
- Codex gets `-c` overrides for a `framerize` provider at
  `<Remote>/openai/v1`. The local token stays in `FRM_LOCAL_TOKEN`.
- `--persist` writes the Claude values to Claude's user settings.
  `--undo` removes them only if they are unchanged since then.
  If `settings.json` is a symlink, both edit its target and keep the link.
  A link with no target fails before anything is written. If the settings
  file is gone, `--undo` reports that Framerize has not written Claude
  settings and forgets its record.
- `--check` prints the OS, client path, version and Remote address.
- `--config FILE` uses that profile for a launch, `--check`, `--persist` and
  `--undo`. `--persist` keeps its undo record beside FILE, so pass the same
  `--config` to `--undo`. If you accept the client offer after setup with a
  non-default `--config`, it ends by printing that profile path for later
  `frm client` commands.

### Install and update

If the client is not on `PATH`, a launch shows the official installer
command: `curl ... | sh` on Linux and macOS, PowerShell on Windows. It runs
only after you type `install`, or with `--install` or `FRM_CLIENT_INSTALL=1`.
Without consent, or with `--no-input` or `FRM_NO_INPUT=1`, it exits with code 3.
Other platforms get the client's download page instead.

`--update` finds the updater that owns the installed client from its path.
The options are the client's own `update` for native installs, `npm install -g`
with the same prefix, or `bun`. It reads the installed version from
`--version` and asks for the latest one: the Claude release channel for a
native Claude install, otherwise `npm view`. If the client is current,
nothing runs. Otherwise it prints the update command. In a terminal it runs
only after you type `update`, even with `--update`, unless `--no-input` or
`FRM_NO_INPUT=1` is set. Any other answer exits with code 3 and changes
nothing. Without a terminal, `--update` is the consent.
Homebrew, mise, winget, Chocolatey and unknown installs get the manual
command or the client's docs instead, with no version check under `--update`.
A failed lookup reports the latest version as unknown, never as current.
During `--check` or `--update`, `SIGINT` or `SIGTERM` cancels pending version
lookups and exits with code 1: `The command was cancelled.` Cancelling a
lookup does not start an update.
These signals also stop an installer or updater that is already running.
A cancelled standalone install or update exits with the same cancellation
diagnostic. Files already changed by the installer or updater remain changed.
On Linux and macOS, cancellation requires `/bin/ps` to find child processes.
Installers and updaters keep the current terminal session for their prompts.

The prompt after an interactive `frm up` checks both clients. It offers an
install for a missing client, and an update only when a newer release is
known. For Homebrew and Chocolatey installs it checks `brew info` or
`choco search` and prints the manual command when a newer release exists.
Each step needs its typed word, and it never starts a client.
Cancelling a question after `frm up` succeeds skips the optional offer and
keeps the successful setup result. The saved connection and service stay in place.
On first-run bare `frm`, cancelling that offer exits successfully without
starting the foreground listener. The saved connection remains available.
Plain `frm setup` also offers client setup after saving the profile.
Cancelling that offer exits with code 1 and `The command was cancelled.`
The saved connection remains available.
`setup --start` runs the same offer after Remote starts. Closing input or
cancelling the pending question with Ctrl+C, `SIGINT` or `SIGTERM` closes
the foreground listener. The offer asks no new questions and starts no
further installs or updates.
During these optional version checks, `SIGINT` or `SIGTERM` also cancels a
pending release request or stops the captured version command. The offer
then skips its remaining output and client operations.
Running installers and updaters also receive cancellation before the offer
returns to its caller.

## Configuration

`up` and `setup` write `~/.framerize-remote/framerize-remote.json`. Set
`FRM_HOME` to an absolute directory to choose another location. The older
`FRAMERIZE_REMOTE_HOME` still works when `FRM_HOME` is unset; `FRM_HOME` wins
when both are set. An explicit `--config FILE` overrides both. Old working-directory profiles
remain usable with `--config /absolute/path/to/profile.json`. Existing profiles
are not moved automatically. The profile stores the remote URL, listen host
and port, the two explicit exposure flags and the token settings. A token from
a pairing link is saved as its value; `setup` saves only file or variable names.

An invalid `FRM_HOME` or `FRAMERIZE_REMOTE_HOME` reports `profile_home_invalid`.
Set it to an absolute directory without control characters, unset it to use
the default, or pass `--config FILE`. `frm help` and `frm --version` work even
when the directory setting is invalid.

Saves are atomic and owner-only on POSIX. New file references become absolute
paths; relative references in existing profiles resolve beside the profile.
Omit `--start` to save without loading credentials or contacting the server.

Setup errors identify a missing server address, invalid token reference,
invalid listen setting or time limit, or a profile that could not be saved.
Each includes a repair step. With `--json`, failures include an error code.
Invalid setup values leave an existing profile unchanged. Error messages
do not include the supplied address, file paths or credential values.

A saved profile or a `serve --host` override can use a non-loopback listen
host without a consent flag. `setup` and `serve` still accept
`--allow-non-loopback` for older commands. Use `setup --help` to change the
saved host.

- `--server-token-env NAME` / `--server-token-file FILE`: optional server
  token. Choose at most one.
- `--local-token-env NAME` / `--local-token-file FILE`: the local client token
  that callers present to this instance. Optional; choose at most one.
  It is never forwarded.
- `--host` / `--port`: listen address, default `127.0.0.1:8788`.
  Port `0` lets the operating system select a port, printed when serving starts.
- `--allow-non-loopback`: accepted for older profiles; no longer required.
- `--allow-insecure-remote`: required for a plain-HTTP remote on a public
  address. Loopback, private LAN, Tailscale `100.64.0.0/10`, `.ts.net`,
  `.local`, `.lan` and single-label hosts need no consent.
  TLS validation is never disabled.

Remote does not auto-load `.env` files and never prints a token. Diagnostics use
fixed messages and never echo raw arguments, remote URLs, bodies, or credentials.

## Commands

- `up [LINK|ORIGIN] [--json]`, or `LINK` alone: save, check and start the service.
- `serve [--json] [--host ADDRESS] [--port NUMBER] [--allow-non-loopback]`: checks readiness before listening.
- `check [--json] [--tailscale]`: reports authenticated readiness; exit 0 ready, 1 otherwise.
- `setup [--start] [--json]`: saves a profile from options; `--start` then serves it.
- `client`: sets up or launches Claude Code or Codex against this Remote.
- `service install|status|start|stop|restart|uninstall`.
- `help`, `version`: no listener, no credential read.

Unknown or repeated options, missing option values and unexpected positional
arguments in `check` or `serve` exit with code 1 and point to command help.
JSON output identifies these syntax errors as `command_arguments`. They are
reported before loading the saved profile and do not echo argument values.

## Temporary listen settings

Use `frm serve --port 8789` to change the local port for one run.
Serve options never rewrite the profile. `SIGINT` and `SIGTERM` shut it down.
Repeated stop signals wait for the same cleanup. Remote prints `Stopped.` only
after cleanup succeeds. A cleanup failure exits with code 1 and explains how
to restart with the same profile. JSON output identifies it as `cleanup_failed`.
To accept connections outside loopback for one run:

```sh
frm serve --host 0.0.0.0
```

A non-loopback `--host` needs no consent flag. Without `--host`, the saved
host applies. Use `--host 127.0.0.1` to restrict an exposed profile
temporarily.

The listener uses HTTP. Protect non-loopback traffic with a private network
or a TLS proxy. Remote does not configure routing, firewall rules or public TLS.
The exposure notice reports the bind policy, not internet reachability.
JSON startup adds `exposure: "non-loopback-http"` only for exposed listeners.

## Endpoints

`GET /health` (unauthenticated), `GET /ready`, `POST /v1/responses`, and
`POST /v1/chat/completions`, all served by the shared core listener. Response
bytes, stream events, and terminal markers pass through unchanged.

Each bare-route inference first checks the remote's authenticated readiness. Failed,
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
inactivity stay bounded at 60 seconds. The response-header timer starts after
the request upload finishes. The local listener also limits the whole forwarding
step to 20 seconds plus the configured inference limit. That overall limit
includes readiness, connection setup and upload, so slow preparation can shorten
the remaining response wait.
An expired inference wait returns 504 without retrying the request.
Existing inference streams have no total lifetime limit after headers arrive.
Client cancellation closes the remote connection. Each exchange owns its
HTTP agent, ignores environment proxies and requires valid HTTPS certificates.

The experimental `/openai/v1/responses` and `/anthropic/v1/messages` routes
forward requests to the same exact server path. Remote sends
no preliminary readiness request for these POSTs. The server validates the body
before accessing its provider credentials. A 404 never triggers a fallback route.
Remote buffers completed JSON replies up to 16 MiB without parsing their JSON.
It forwards `text/event-stream` replies as bytes arrive, including the server's
keepalives. Protocol conversion belongs to the server, not Remote.
The inference deadline covers the body in both modes. Broken or oversized JSON
exchanges fail before success. A broken stream closes without a success terminator.
These routes have local synthetic coverage, not live provider qualification.
Streaming clients use `stream: true`; completed-JSON clients use `stream: false`.
The server still makes a single non-streaming request to its custom provider
endpoint. T3-specific adapters and live provider acceptance need separate checks.

Startup output is compact by default. JSON mode covers setup, readiness,
listening, shutdown and command failures. Failed checks identify missing
profiles, the credential role, DNS/TCP/TLS failures, deadlines, HTTP refusals,
or invalid readiness data. Each diagnostic includes one suggested action.
JSON keeps `ready` and `reason`, with an added `diagnostic` on remote failures.

`frm check --tailscale` also runs local `tailscale status --json`.
It reports a fixed status label without machine names, IP addresses or login
links. It never logs in or changes Tailscale Serve, Funnel or other settings.
The subprocess has a 1.5-second deadline and a combined 256 KiB output limit.
Cancellation and limit failures terminate and reap it before returning.
Its status is advisory and does not change the readiness exit code, except
that cancellation exits nonzero. Local daemon status does not prove remote
reachability. Remote still needs an accessible server origin and valid token.

Browserless provider and Castle runtime integration, native delivery and
release CI migration remain separate unfinished work.
