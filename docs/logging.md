# Framerize event logging

Structured observability, following the standard checklist: NDJSON events, one
**canonical wide event per operation** (not a spray of step logs), request IDs,
standard field names, no secrets, retention, and a query interface.

## Where events live

`<profiles>/.logs/events-YYYY-MM-DD.ndjson` (daemon default:
`~/.framerize/profiles/.logs/`). The daemon and the Camoufox sidecar write the
**same schema into the same directory** (the sidecar gets the dir via
`FRAMERIZE_LOG_DIR`). Daily files; files older than 7 days are deleted at
daemon start.

Native serving initializes the same sink after selected-account validation.
It preserves the profile's filesystem path, including non-UTF-8 names. If the
daily log cannot open, a fixed warning goes to stderr once and requests keep
running. Later events retry opening it; an obstructing file is never replaced.
Events dropped before the destination becomes available remain lost evidence.

The Rust writer serializes each event, including its newline, before one append
attempt. It does not stream JSON fragments into the shared file or retry a short
write with a suffix that could follow a sidecar record. A short write or error is
lost evidence and is ignored by serving; consumers must still reject missing or
malformed records. This is not a cross-process lock, a durability guarantee, or
an atomicity promise for every filesystem. Existing sidecar writing is unchanged.
The core and focused fixture compilations pass. All five fixtures pass, including
independent append handles and short-write/error paths. The running service
keeps its existing writer until a separately validated package is activated.

## Schema

One JSON object per line. Standard fields on every event:

| field | meaning |
|---|---|
| `ts` | RFC3339 UTC timestamp |
| `level` | `info` \| `error` |
| `event` | dot-namespaced event name (see table) |
| `request_id` | uuid correlating everything caused by one inbound request |
| `account` | first 8 chars of the account uuid |
| `duration_ms` | wall time of the operation |
| `outcome` | `ok` \| `error` |
| `source` | `sidecar` when emitted by the python bridge (daemon omits it) |

Event-specific fields are flat extras (`status`, `chunks`, `duration_ms`...).

## Events

| event | key fields | meaning |
|---|---|---|
| `daemon.start` | `listen`, `engine` | daemon boot |
| `login.begin` | `email` (masked), `duration_ms` | magic-link signin, canonical |
| `login.complete` | `duration_ms` | token exchange + account registration |
| `account.import` | `duration_ms` | session-cookie import |
| `camoufox.launch` | `duration_ms` | engine cold start (sidecar) |
| `castle.mint` | `kind` (`v3`\|`v2`), `token_len`, `duration_ms` | token minted (value NEVER logged) |
| `auth.signin` / `auth.complete` | `status`, `duration_ms` | in-page auth calls (sidecar) |
| `ai.request` | `dialect`, `streaming`, `status`, `duration_ms` | one gateway call through the proxy |
| `ai.chain` | `model`, `dialect`, `chunks`, **`metered_chunks`**, `prompt_tokens`, `completion_tokens`, `finish_reason`, `duration_ms` | **canonical free-chain summary** |
| `ai.post` | `path`, `status`, `duration_ms` | the sidecar's in-page POST |
| `sidecar.respawn` | — | dead sidecar restarted |
| `credits.refresh` | `available` | ledger read |

## Credit accounting (the provable-free view)

`ai.chain.metered_chunks` counts how many chunks fell back to the metered
stream path. `0` = the whole generation rode the free non-streaming route.
Ledger truth comes from `credits.refresh` events (or `/api/accounts`):
`available` before/after a window shows the real delta.

```bash
framerize logs --credits          # AI events: metered_chunks, tokens, durations
framerize logs --event ai.chain --stats
```

## Querying

```bash
framerize logs                    # last 50 events
framerize logs --tail 200
framerize logs --event login      # substring filter
framerize logs --account 86a968f3
framerize logs --stats            # count + avg duration by event
framerize logs --credits
```

## Private Claude body capture

The browser host can capture the prepared `/ai/v3/messages/` request and its
returned response for diagnosis. This is off by default. To enable it, create
`<FRAMERIZE_LOG_DIR>/claude-bodies` with mode `0700` and an empty `enabled`
file inside it with mode `0600`. The marker enables capture for two hours
after its modification time. Removing it disables new captures immediately;
already queued writes may finish. No restart is needed after this code is active.

Each record has a job ID, timestamp and phase. Request bodies are observed
before browser dispatch; a recorded request alone does not prove it was sent.
Response records preserve status and network failure facts. A failed browser
fetch has `complete_response: false`; its error text is not an upstream body.
If the browser cannot read an HTTP 504 body, capture cannot recover that body.

Capture retains prompt text, tool inputs and returned content in private files.
Authentication fields, bearer values and JWTs are redacted. Native thinking
blocks and their signatures remain available for history comparison. Captures
must stay outside source control and must not be copied into ordinary logs.

Each record is capped at 8 MiB. Larger bodies produce an explicit omission
record. The writer retains at most 128 files and 64 MiB, with two queued
records and one active write. A full queue drops diagnostics without waiting.
Write failures do not interrupt inference. Existing browser debugger request
and response body capture remains disabled, so this does not restore its
per-page retained payloads. The 55 focused dispatch, response-ownership,
network-diagnostic and capture checks pass. Real-service capture qualification
is still pending.

## Ordinary event rules

- **Never log secrets**: no castle tokens, session cookies, JWTs, or prompt
  text. Token *kind* and *length* only. Emails masked (`j***@gmail.com`).
- One canonical event per operation; phases as fields, not separate lines.
- Logging must never break the service: emit failures are swallowed.
- Multi-day daemon runs roll the file on restart (documented simplification).
