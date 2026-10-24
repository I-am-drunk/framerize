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

The Rust writer serializes each event, including its newline, before one append
attempt. It does not stream JSON fragments into the shared file or retry a short
write with a suffix that could follow a sidecar record. A short write or error is
lost evidence and is ignored by serving; consumers must still reject missing or
malformed records. This is not a cross-process lock, a durability guarantee, or
an atomicity promise for every filesystem. Existing sidecar writing is unchanged.
This correction and its five focused fixtures are source-only and unrun pending
the coordinated session reload. The running service keeps its existing writer.

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

## Rules

- **Never log secrets**: no castle tokens, session cookies, JWTs, or prompt
  text. Token *kind* and *length* only. Emails masked (`j***@gmail.com`).
- One canonical event per operation; phases as fields, not separate lines.
- Logging must never break the service: emit failures are swallowed.
- Multi-day daemon runs roll the file on restart (documented simplification).
