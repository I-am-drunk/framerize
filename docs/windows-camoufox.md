# Running Framerize on Windows (Camoufox)

## Why Camoufox

On a fresh Windows device, `--engine full-chrome` (desktop Chrome/Edge) and
`--engine headless-shell` are refused by Framer's Castle device checks. Every
inference request comes back:

```
403 {"error":{"message":"This request was blocked by our security checks",
              "data":{"reason":"request_blocked"}}}
```

`--engine camoufox` drives the stealth Firefox build (Camoufox) that clears the
Castle device check. It is the engine this project documents as the one that
passed Castle in production.

## Prerequisites

- Python 3.12 with the pinned sidecar deps (`runtime/requirements.txt`):
  `camoufox==0.5.6`, `playwright==1.62.0`.
- The Camoufox browser binary: `python -m camoufox fetch`.
- Build the daemon and web UI:
  `cargo build -p framerize`, then in `web/`: `npm ci; npm run build`.

## Run

```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows\run-daemon.ps1
```

The launcher:

- resolves `FRAMERIZE_PYTHON` to the real Python interpreter. Windows has no
  `python3`; the Microsoft Store `python3.exe` alias cannot import packages.
- points `--sidecar-script` at the cross-platform `scripts/camoufox_host.py`.
  The bundled `runtime/framerize_host` package imports `fcntl` and is Unix-only.
- stores the database, session cookies and per-account browser state under
  `.framerize/runtime` (gitignored) so imported accounts survive restarts.

Verify readiness and run a completion:

```powershell
curl.exe http://127.0.0.1:8787/api/ready
curl.exe -X POST http://127.0.0.1:8787/openai/v1/chat/completions `
  -H "content-type: application/json" `
  -d '{\"model\":\"gpt-4.1\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}],\"max_tokens\":32}'
```

The same account serves the Anthropic-compatible route:

```powershell
curl.exe -X POST http://127.0.0.1:8787/anthropic/v1/messages `
  -H "content-type: application/json" `
  -d '{\"model\":\"claude-haiku-4-5\",\"max_tokens\":32,\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}'
```

## Two fixes required for Windows

1. `scripts/camoufox_host.py` — added the `history_status` readiness command the
   Rust sidecar probes at startup. Without it `/api/ready` fails with
   `unknown cmd history_status` (reason `account-history-unavailable`).
2. `crates/framer-host/src/sidecar.rs` — `with_script` required
   `canonicalize(path) == path`, but Windows `canonicalize` returns a
   `\\?\C:\...` verbatim path, so no normal absolute path could ever pass. It
   now accepts the same file addressed without the verbatim prefix (relative and
   unresolved paths are still rejected).

## Notes

- The Responses dialect (which both compat routes translate to) requires an
  output budget of at least 16 tokens. `max_tokens < 16` returns
  `Responses output budget must be at least 16 tokens` (surfaced as HTTP 500).
- Free-route requests (`/openai/v1/chat/completions`, `/openai/v1/responses`,
  `/anthropic/v1/messages`) do not change the imported account's credit
  balance. The `usage.cost` field is the platform's upstream cost, not a charge
  to the account.
- A saved Framer session imports only if at least one of its workspaces has
  credits; otherwise the import responds
  `account not eligible: <alias>: no workspace has credits - account cannot serve requests`.

## Importing saved sessions

```
POST /api/accounts/import
{"account_id":"<uuid>","session_cookie":"session=<uuid>","alias":"<name>",
 "team_id":"<uuid>","project_id":"<id>"}
```

Import probes the live `/web/users/me` endpoint, claims a credited workspace and
starts serving the account with the Camoufox device on first use.
