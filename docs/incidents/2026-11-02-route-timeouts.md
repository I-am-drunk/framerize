# Custom route timeout evidence, 2026-11-02

The user recalled selecting a custom route for an unmetered 180-second or
1,080-second window. Preserve requested reasoning effort while checking that
hypothesis. A local deadline, observed response duration and upstream limit
are different measurements.

## Current routes

Anthropic clients use `/ai/v3/messages/`. OpenAI Responses and Chat clients
use `/ai/v3/responses/`. Both are custom OpenRouter routes with buffered
upstream requests. `/ai/v3/openai/responses/` is the direct metered route.
Alpha.11 keeps paid routes and paid fallback disabled in production.

The original native Responses change, `3b33d50f` on 2026-10-06, selected the
Responses route to preserve OpenAI tools, reasoning items and response
semantics. Its recorded duration proof was 3,311 tokens in 24.3 seconds.
That observation does not establish an upstream timeout ceiling.

## Local deadlines

`runtime/framerize_host/app.py` derives a buffered response deadline from
the requested token allowance, with a 60-second floor and 180-second ceiling.
This applies to both custom routes. The 840-second streaming ceiling applies
only when upstream streaming is requested; production custom requests do not
use that metered transport. The enclosing host RPC bound is 900 seconds.

At `3b33d50f`, the old sidecar allowed a 900-second response ceiling. It read
only `max_tokens`, so native Responses requests without that field used the
600-token default, yielding a 180-second local deadline. The current sidecar
also reads `max_output_tokens`. No 1,080-second inference limit was found in
the inspected route code or its Git history. This does not establish whether
an external client had such a setting.

Changing a local deadline cannot extend a timeout enforced upstream.

## Observed failures

The isolated Claude capture recorded upstream HTTP 504 after 60.122 seconds.
Total request time was 105.951 seconds because preparation took additional
time. This is direct evidence of an upstream failure on `/ai/v3/messages/`.

A frozen read of the November 1 and 2 production logs found:

- Messages: 34 fetch failures, all after 60 seconds, and 11 successes. The
  longest successful operation took 57.627 seconds.
- Responses: 36 fetch failures, including 13 after 60 seconds, and 17,370
  successes. The longest successful operation took 57.773 seconds.

Those production durations exclude recorded queue time but still include
preparation. Older Responses operations sometimes exceeded 60 or even 180
seconds. Success events do not retain their separate fetch duration, so those
records cannot establish an upstream survival window. The older failures
likewise lack the newer browser HTTP-status observation. Do not describe them
as confirmed Responses HTTP 504s or infer an exact universal 60-second limit.

## Decision and remaining proof

Preserve requested effort and the existing custom routes. A longer Responses
window is unproven. The legacy Anthropic-to-Chat converter also omits thinking
blocks and settings, so composing it with Chat-to-Responses would lose client
semantics. A route switch requires both a demonstrated timing benefit and
preservation of thinking, signatures, tools and output limits.

Alpha.11 already records request-scoped browser HTTP failures and prevents
replay of uncertain adaptive Claude work. It remains the running binary.
This investigation changes documentation only and sends no inference probes.
Its earlier passing package and live-delivery evidence remains unchanged.

The read-only recheck returned HTTP 200 from health and readiness. PID 2577096,
its 02:03:01 UTC start time and zero automatic restarts were unchanged.
The running executable still hashes to
`aab320ae6667d47dc2facb2dbc1ec8159aa880fe0f11a9d5fc0840e8c4ce1ce5`.

Private evidence is under
`/home/tjninja/.cache/framerize-claude-stalls-20261101/route-timeouts-r1`:
`route-events.json`, `historical-route-events.json`, `journal-failures.json`
and `live-after.json`.
The exact Claude capture and activation evidence are linked from
[the preceding incident](2026-11-01-claude-stalls.md).

Next proof: retain the browser status and elapsed time from the next ordinary
custom Responses failure. The exclusive isolated workspace remains out of
credits; its earlier 402 is not a successful delivery or a timeout measurement.
