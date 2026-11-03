# Historical native-chain design, October 2026

This is an archived design and measurement record. Its retry counts, model
selectors and file names do not describe the current implementation. Use
[native chains](native-chains.md) for the current contract.

Status: implemented 2026-10-06. This document defines how long generations are
chained on each FREE upstream route, per client dialect, with zero
cross-dialect conversion for OpenAI clients.

## Why chains exist

The free-route exploit requires **non-streaming** upstream requests (the meter
only hooks SSE). A non-streaming request emits no bytes until the generation
completes — and the network path kills any connection idle for a window:

| route | measured idle window |
|---|---|
| `/ai/v3/chat/` (OpenRouter chat) | ~21.5s |
| `/ai/v3/anthropic/messages/` (direct) | ~60s |
| `/ai/v3/messages/` (OpenRouter anthropic-format) | ~45–109s |
| `/ai/v3/responses/` (OpenRouter responses) | ≥24s confirmed; upper limit not established by the 2026-10-06 capture of 3311 tokens in 24.3s |

These are historical observations, not current service guarantees. The
[2026-11-02 investigation](incidents/2026-11-02-route-timeouts.md) separates
upstream failures from local deadlines. It confirms a Claude upstream 504 at
60.122 seconds and records recent Responses fetch failures around 60 seconds,
without claiming an exact Responses ceiling. The local buffered deadline is
at most 180 seconds on both custom routes.

So any generation that might exceed the window must be **chained**: fired as a
sequence of window-safe non-streaming chunks, continued until the model stops.
This applies only where continuation preserves the protocol. Adaptive Claude
thinking cannot be resumed with a synthetic continuation; alpha.11 reports
its incomplete-response failure without replay or paid fallback.

## The dialect-native rule

Every client dialect runs on its own native free route. **No OpenAI-format
request is ever converted to Anthropic format.** (The old Responses→anthro
conversion was the root cause of the "no tools available" class of bugs —
custom tools, reasoning items, service tiers, and response ids all survive
only on the native path.)

| client dialect | upstream route | chain |
|---|---|---|
| Anthropic Messages (Claude Code) | `/ai/v3/messages/` | `freechat.rs` (existing) |
| OpenAI Responses (Codex) | `/ai/v3/responses/` | `nativechain.rs` |
| OpenAI chat/completions | `/ai/v3/responses/` via a thin boundary mapping | `nativechain.rs` |

`/ai/v3/chat/` is retained in the route registry but unused in production —
its 21.5s window is strictly worse and chat clients map losslessly onto the
Responses route instead (both are OpenAI-native shapes).

## The responses-native chain (`nativechain.rs`)

### Chunk construction
- Payload = the client's request **verbatim**, with only: `model` normalized
  (OpenRouter prefixed/dotted id rules), `stream` forced off,
  `max_output_tokens` set to the chunk budget, and `store` /
  `previous_response_id` / `background` **stripped** — the proxy 400s all
  three (measured 2026-10-06: "not supported on this proxy"). They are
  persistence hints, not semantics, so stripping is lossless.
- Budget sizing is identical policy to freechat: ramp from 600 tokens,
  adapt to `measured_tps × 45s` clamped 400–3000, queue-time excluded from
  the throughput measurement, 300s wall-clock cap, 0.7–2.3s inter-chunk
  jitter, two-consecutive-slow-chunks (<8 tok/s) fail-fast.

### Finish detection (per chunk)
- `status: "completed"` → done.
- `status: "incomplete"` + `incomplete_details.reason: "max_output_tokens"`
  → continue.
- any other incomplete reason → done honestly (surface the reason).

### Continuation
`previous_response_id` is unsupported (measured), so history is carried
client-side: append the chunk's **entire `output` array verbatim** to `input`
(reasoning items included — verified 200 on a chained astra turn), then append
a user message: "Continue exactly where you stopped. Do not repeat anything."

### Mid-tool-call cut
If the chunk ended at max_output_tokens with the last output item a
`function_call` or `custom_tool_call`, the call's arguments/input are
truncated and cannot be continued literally. Policy (same as freechat):
1. ≤2 free retries with a window-proportional budget (`tps × 70s`,
   clamp 1200–5000) and an explicit split instruction appended as a user
   message ("write at most ~40 lines per call, then append").
2. If still cut: metered-stream fallback for that chunk only when the
   operator opted in (`FRAMERIZE_ALLOW_METERED=1`); otherwise an honest
   error. Auto-refund fires on metered chunks (quota permitting).

### Assembly
- Consecutive assistant `message` items merge into one logical message
  (output_text parts concatenated in order).
- `reasoning`, `function_call`, `custom_tool_call` items append as-is.
- Final object: `id` from the FIRST chunk (client correlates on it),
  `status: "completed"`, merged `output`, `usage.input_tokens` = last chunk's
  (history grows; latest is truth), `usage.output_tokens` = SUM,
  `service_tier` lifted from the last chunk that reports one.

### Client streaming (synthesized SSE, never a silent socket)
Per chunk, its new output items stream to the client immediately:
- message (continued text): one open `output_item` across chunks — text
  deltas keep flowing; the item only closes when the message completes.
- function_call → `response.function_call_arguments.delta/.done`
- custom_tool_call → `response.custom_tool_call_input.delta/.done`
- reasoning → passed through as reasoning items.
`response.completed` carries the assembled object. Non-streaming clients get
the assembled JSON — semantically a pure passthrough.

## Chat-dialect boundary (thin, OpenAI→OpenAI)

Request mapping (chat → responses): system→developer message, user/assistant
messages → message items (assistant content → output_text), `tool` messages →
`function_call_output`, assistant `tool_calls` → `function_call` items,
`tools[].function` flattened to responses tools, `tool_choice` function form
flattened, `max_tokens|max_completion_tokens` → `max_output_tokens`,
`response_format.json_object` and `response_format.json_schema` → `text.format`
(schema name, description, constraints and supplied strictness preserved), `stop`/temperature/top_p
passthrough; dropped (documented): seed, logprobs, n≠1 (error).

Response mapping (assembled responses → chat.completion): merged text +
function_call items → `choices[0].message{content, tool_calls}`,
finish_reason = `tool_calls` if any calls, `length` if the chain hit its
budget cap, else `stop`; usage mapped prompt/completion/total. Streaming:
`chat.completion.chunk` frames from the same item events + `[DONE]`.

## Metered-route policy (user rule, 2026-10-06)

The direct route `/ai/v3/openai/responses/` (sol/luna/terra — metered
always) is **blocked by default**: `model_route` returns
`Unavailable("metered-only model … FRAMERIZE_ALLOW_METERED=1 to allow")`
unless the env var is set. No silent credit burn, no substitution. The
credit-bond floor still guards the allowed case.

## Failure policy (all chains)

- 429: instant retry (≤4 in-chain) + `on_429` callback → workspace marked
  saturated 45s for rotation steering; then honest error.
- 403 fast: velocity classification lives in the sidecar (device cooling +
  rotation); chains see the final status.
- Upstream content refusal: terminal for the chunk (no retry).
- Wall-clock: 300s cap; sustained-collapse detector fails fast with
  "upstream degraded — retry in a minute".

## Speed-tier investigation (2026-10-07, measured)

Suspected "ultrafast feels normal" — full decomposition, all measured live:
- No `:nitro` anywhere in the stack (it admits priority only, never ultrafast).
  The T3 `-ultrafast` variants inject `service_tier: "ultrafast"` — the correct
  opt-in — and the chain preserves it verbatim on every chunk.
- Phase-separated decode timing on astra ultrafast (2,833 output tokens):
  **text decode 570 tok/s** (faster than OpenRouter's ~350 public figures);
  **reasoning phase 191 tok/s** — 2,070 invisible reasoning tokens = 75% of
  wall time. The felt slowness is the model's reasoning phase, not the tier,
  not the provider, not the chain.
- Tier-aware chain sizing deployed: fast-lane requests (service_tier
  ultrafast/priority/fast) start chunks at 1500 (not 600) and grow to fill
  the 45s window at measured speed (cap 12000 vs 3000) — near-single-shot,
  killing the per-chunk TTFT+continuation overhead (~2-3s × N chunks).
- Provider pinning tested: no consistent win, routing left to OpenRouter.
- Reasoning effort is the user's lever: the gateway passes `reasoning` /
  `reasoning_effort` verbatim (never auto-injects). Set it in codex config
  (`model_reasoning_effort`) or the request body.
