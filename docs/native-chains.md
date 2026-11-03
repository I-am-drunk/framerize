# Native request chains

This describes PR #125's source as of 2026-11-03. Exact-package qualification
is pending. The [October design](historical-native-chains-20261006.md) retains
the earlier measurements; it is not the current request contract.

## Routes and shared controls

| Client dialect | Custom upstream | Implementation |
| --- | --- | --- |
| Anthropic Messages | `/ai/v3/messages/` | `crates/framer-api/src/chain/messages.rs` |
| OpenAI Responses | `/ai/v3/responses/` | `crates/framer-api/src/chain/responses.rs` |
| OpenAI Chat | `/ai/v3/responses/` with boundary conversion | Same Responses chain |

The routes share account ownership, admission, cancellation, active-time
accounting, output admission capacity and error conversion. Each dialect
retains its protocol-specific continuation and stream encoder. A custom-route
request cannot silently select a paid upstream or another model.

The browser's local buffered deadline is at most 180 seconds on both custom
routes. An upstream gateway can fail earlier. A captured Claude HTTP 504
arrived after 60.122 seconds. Changing the local deadline does not extend that
gateway's deadline. See the [timeout investigation](incidents/2026-11-02-route-timeouts.md).

## Claude continuation

Every Messages model and thinking mode uses the same implementation. Native
provider fields, system content, tools and signed blocks are preserved.
The first buffered request allows 600 tokens; subsequent allowances target
45 seconds from observed throughput, normally within 400 to 1,200 tokens.
An explicit thinking budget raises the minimum to its budget plus one token.
The caller's total token allowance and requested effort remain unchanged.

Continuation requires a complete acknowledged reply with usable content.
Signed history stores hashes and block offsets before publication, then
restores original assistant-turn boundaries on an exact matching follow-up.
Incomplete client tool calls are not published. Recovery keeps only the exact
prefix before the first unpublished tool and allows two smaller-call retries.
Server-tool truncation and unknown transport completion cannot use that path.

The chain retains its chunk limit and 300-second active-time limit. Cooldown
waits do not consume active time. A token allowance is an upper bound, not a
guarantee that the provider can complete every request before its deadline.

## Errors and stream completion

Known temporary Framer security refusals record the cooldown and retain the
same account and unfinished chunk. Initial admission and later waits share a
960-second server budget. The stream sends pings during silence. Disconnect
prevents another attempt; already-sent work retains cleanup ownership.
Initial waits hold no account or model permit. Admitted Messages requests
retain both across later waits. Speed or an empty 403 body alone never
authorizes a security retry.

Permanent provider errors retain their HTTP status and Anthropic error type.
Once streaming starts, failures use a terminal SSE error event without a
success terminator. Partial output and unknown completion disable whole-request
replay. SSE error events cannot force a client SDK to retry automatically.
Queue overflow reports an incomplete response instead of silently losing text.

Messages and Responses reserve capacity from the same output owner. Retained
Messages byte buffers keep that reservation until their last clone is dropped.
The [reliability audit](incidents/2026-11-03-claude-reliability.md) records tests,
remaining live checks and the required history-compatible rollback package.
