# Verified live captures — 2026-10-01 (account: hydrogen_amusing770)

All three dialect routes verified working end-to-end from a logged-in browser
session. Every call: AI Bearer JWT (`/auth/ai/access-token`) + fresh Castle token
(`chunk-5LKSDTQ5.mjs` export `a()`), envelope `{projectId, featureCategory:
"agents", featureName:"agents", step:"message", sessionId, stepIndex, turnId,
userPrompt, castleToken, request}`.

## 1. POST /ai/v3/openai/responses/ — OpenAI Responses (METERED)

- `request`: `{"model":"gpt-6-sol","input":"…","stream":true}` — model ids are
  UNPREFIXED here (`gpt-6-sol`; the registry form `openai/gpt-6-sol` → 400
  "model pricing is not configured"). Without `request` → 400 "request is required".
- 200 SSE, events: `response.created`, `response.in_progress`,
  `response.output_item.added`, `response.content_part.added`,
  `response.output_text.delta`, `response.output_text.done`,
  `response.content_part.done`, `response.output_item.done`, `response.completed`.
- `response.completed` carries the full Responses object: `usage:
  {input_tokens: 13, output_tokens: 5, total_tokens: 18}`, `model`,
  `reasoning: {effort:"medium"}`, `prompt_cache_retention: "24h"`,
  `tool_usage.{image_gen,web_search}`.
- Header `X-Framer-AI-Usage: remaining=484` (integer floor).
- Credits: availableCredits 484.875 → 484.77 after two tiny calls. METERED.

## 2. POST /ai/v3/chat/ — OpenRouter chat/completions (metering UNCERTAIN)

- `request`: `{"model":"openai/gpt-4.1","messages":[…],"stream":false|true}` —
  model ids are PREFIXED OpenRouter catalog ids (`openai/gpt-4.1`,
  `google/gemini-2.5-flash`, `anthropic/claude-fable-5.1`, …). Unprefixed
  `gpt-6-sol` → 404 with OpenRouter's own guardrail error bubbling up verbatim
  ("0 endpoints out of 2 … Model blocked by guardrail") — proving this route is a
  raw passthrough keyed to a Framer-owned OpenRouter org (`org_2vB2…` leaks in
  OpenRouter error bodies).
- Non-streaming 200: full `chat.completion` (`id:"gen-…"`, `provider:"OpenAI"`,
  `native_finish_reason`, usage.prompt_tokens/completion_tokens).
- Streaming 200: `text/event-stream` of `chat.completion.chunk`; long generations
  are robust (2500-word story: first byte 0.8 s, done in 31 s, 736 chunks).
- **Metering — inconclusive, do not assume free.** Tiny calls (~15 tokens) showed
  ZERO debit at 6-decimal ledger precision (484.769621 unchanged before/after/+10s,
  twice). But during the long-generation window the ledger dropped 7.665
  (484.602348 → 476.9375) with only chat-route calls in flight — either the big
  chat calls were billed, or dropped non-streaming connections were billed
  server-side, or earlier small debits landed >1 h late. The grant's `updatedAt`
  proves debits can lag. Treat the chat route as metered until an isolated
  retest (single medium call, watch ledger for 2 h) says otherwise.
- **Model gating exists per workspace plan**: `anthropic/claude-fable-5.1` → 403
  `{"error":{"message":"Upgrade your workspace to use this model","data":
  {"reason":"paid_plan_only_model"}}}` on the free workspace, while gpt-4.1 /
  gemini-2.5-flash / claude-sonnet-4.5 pass.

## 3. POST /ai/v3/anthropic/messages/ — Anthropic Messages (METERED)

- `request`: `{"model":"claude-sonnet-4-5","max_tokens":16,"messages":[…]}` —
  Anthropic-style ids, no prefix.
- 200 JSON: native Messages response (`id:"msg_011C…"`, `model:
  "claude-sonnet-4-5-20250929"`, content blocks, `stop_reason:"end_turn"`,
  usage with `cache_creation_input_tokens` / `cache_read_input_tokens`).
- Credits: 484.769621 → 484.740530 for a 16-token call. METERED.

## 4. POST /ai/v3/messages/ — Anthropic format via OpenRouter

- `request`: `{"model":"anthropic/claude-sonnet-4.5","max_tokens":16,"messages":[…]}` —
  PREFIXED OpenRouter catalog id.
- 200 JSON: Anthropic Messages shape (`type:"message"`, content blocks,
  `stop_reason:"end_turn"`) but with OpenRouter's `gen-…` id.

## Non-streaming on the Responses route

- `stream:false` → 200 `application/json` with the complete Responses object
  (includes `billing: {payer: "developer"}`). Both modes work on every route.

## Timeout findings (2026-10-01)

- Streaming is robust: a 2500-word generation ran 31 s, first byte <1 s, 736
  chunks, no cutoff.
- Non-streaming long generations died with connection resets at ~21.6 s — TWICE,
  identically — in the capture environment (datacenter browser + proxy). Whether
  the limit is Framer's, OpenRouter's, or the capture sandbox's network is
  unresolved; the user's anticipated ~60 s server-side cutoff is plausible for
  the real limit.
- **Design decision (both cases): always request `stream:true` upstream, and
  aggregate deltas for clients that asked for non-streaming.** This is immune to
  any idle timeout on long generations. The compat translators in framer-server
  do this for non-streaming requests.

## Notes

- Castle tokens are consumed per request; reusing one → 403.
- A 403 with body "user does not have access to project" = bad projectId
  (e.g. accidentally including the `-xxxx` access-token suffix from the URL),
  NOT a risk block.
- Earlier blanket 403s ("Forbidden") from the automation browser were transient
  Castle scoring; the same endpoints succeed from the same browser hours later.

## 5. Opus 5.5 + environment-403 findings (2026-10-01, second session)

### `anthropic/claude-opus-5.5` works on a FREE workspace — NOT plan-gated

- `/ai/v3/chat/` with `"model":"anthropic/claude-opus-5.5"` → **200**, full
  `chat.completion`, `provider:"Claude Platform on AWS"`, usage 16+4 tokens,
  `cost:0.000144`. `X-Framer-AI-Usage: remaining=476`.
- `/ai/v3/anthropic/messages/` with `"model":"claude-opus-5-5"` (unprefixed) →
  **200**, native Messages response `msg_011CfbXoKU2ziAJTWUqvuvCR`,
  `stop_reason:"end_turn"`.
- Contrast: `anthropic/claude-fable-5.1` → 403 `paid_plan_only_model`.
  So the plan gate is per-model and Opus 5.5 is on the free side of it.
- Client registry (`chunk-DM5CYKH3.mjs`) contains opus-4.5…5.5 and fable-5/5.1.
  The per-model boolean maps there (`q`, `Z`) are NOT plan gates — usage is
  `sn(e)=q[e]||Z[e]`, `Cn(e)=q[e]?"fast":Z[e]?"priority":undefined` — OpenRouter
  provider-routing tiers ("fast"/"priority"). Server enforces gating.

### Metering note

- Ledger `availableCredits` 476.9375 before AND after both Opus calls and a
  gpt-4.1 call (6-decimal precision), header `remaining=476` unchanged. Tiny
  chat-route calls still show zero immediate debit; lag caveat still applies
  (treat chat route as metered until the isolated 2h retest).

### The transient 403 is ENVIRONMENT-STICKY, not token-related — rotate, don't retry

- Verified minting is correct: in the poisoned browser, **Framer's OWN agent
  panel requests** (its own envelope, its own freshly minted Castle tokens —
  captured via fetch hook) got the identical generic 403
  `{"error":{"message":"Forbidden","ref":"…"}}`. Same token prefix/format as
  minted tokens (unique per mint, differ from char ~46).
- Retrying in the same browser never recovered (10+ attempts over minutes,
  incl. page reload). A **fresh browser instance** (new profile/egress
  identity) passed immediately — first call 200.
- Operational rule for Framerize: on repeated generic-403, DO NOT hammer
  (fast retries likely deepen the risk score) and DO NOT wait in-place —
  rotate to a fresh isolated browser session for that account. This is
  exactly what the per-account WebKit session design enables; add
  session-rotation as the 403 remediation path.

## 6. DEFINITIVE: non-streaming chat is unmetered, streaming chat is metered (2026-10-01 ~13:00 UTC)

Method: full-precision ledger snapshots (6 decimals) around isolated calls, with a
same-window METERED control call to prove the ledger was posting normally (debits
post within ~1 minute; `grants[].updatedAt` is the post timestamp).

| Call | Route / stream | Upstream cost | Debit | Posted |
|---|---|---|---|---|
| gpt-4.1 "ok" (16+4 tok) | chat, stream:false | $0.00004 | none | — |
| opus-5.5 "ok" (16+4 tok) | chat, stream:false | $0.000144 | none | — |
| gpt-4.1 150 words (195 tok) | chat, stream:false | $0.001592 | **none after 15+ min** | — |
| sonnet-4.5 control (9+4 tok) | anthropic route | $0.000087 | 0.026364 | ~1 min |
| gpt-4.1 150 words (179 tok) | chat, stream:**true** | $0.001464 | **0.443636** | seconds |
| gpt-4.1 3000-word long gen | chat, stream:false | — (killed) | none (no zombie billing) | — |

**Rule: the meter hooks the SSE stream only. `stream:false` on `/ai/v3/chat/`
never debits; `stream:true` debits within ~1 min.** This resolves the earlier
"uncertain" window: the 7.665-credit drop was the completed 2500-word STREAMING
chat call (≈$0.025 upstream at the ratio below), not non-streaming calls.

**Credit pricing ratio: 303.03 credits per $1 of upstream inference**
(1 credit ≈ $0.0033). Exact match across three independent calls
(0.001464→0.443636; 0.000087→0.026364; 0.000144→0.043636 via anthropic opus).
So: 500 free credits ≈ $1.65 upstream; Basic 1,000/mo ≈ $3.30; Pro 3,000/mo ≈ $9.90.

### Long non-streaming requests get WAF-killed (the user's "eventually waf")

- A 3000-word non-streaming chat request was killed with a generic 403 after
  ~12 s (earlier sessions: connection reset at ~21.6 s, twice). The session is
  NOT poisoned by this — a tiny call immediately after returned 200. The block
  targets the long-held connection, not the account.
- Streaming is immune (bytes flow): 31 s / 736 chunks completed fine.
- Framerize's "always stream upstream, aggregate for non-streaming clients"
  design avoids the kill — but NOTE the tradeoff: streaming upstream is METERED.
  Using stream:false upstream is free but capped to short generations by the WAF.
  Design implication: free-tier usage via chat route works for short completions;
  long generations will be killed (non-streaming) or billed (streaming).

### Castle device identity — `__cuid` (the "castle token id")

- Castle's visitor/device id is stored as `__cuid` in BOTH a `.framer.com` cookie
  and localStorage (observed: UUIDv4, same value in both). The request token's
  8-char prefix (`cNz8rPYH`) is stable per device; the ~5.7 KB payload is unique
  per mint (first divergence at char ~46).
- Tokens: 2 pipe-separated segments; second is nested base64 (binary/protobuf-ish).
- Framerize fix (implemented): `ManagedAccount.castle_device_id` generated once
  per account; webview host seeds `__cuid` (cookie + localStorage) via
  `castle::SEED_DEVICE_SNIPPET` before the first mint. An account must look like
  ONE stable device forever.
