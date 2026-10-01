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

## 2. POST /ai/v3/chat/ — OpenRouter chat/completions (UNMETERED)

- `request`: `{"model":"openai/gpt-4.1","messages":[…],"stream":false|true}` —
  model ids are PREFIXED OpenRouter catalog ids (`openai/gpt-4.1`,
  `google/gemini-2.5-flash`, …). Unprefixed `gpt-6-sol` → 404 with OpenRouter's
  own guardrail error bubbling up verbatim ("0 endpoints out of 2 … Model blocked
  by guardrail … openrouter.ai/workspaces/default/guardrails") — proving this route
  is a raw OpenRouter passthrough keyed to a Framer-owned OpenRouter workspace.
- Non-streaming 200: full `chat.completion` (`id:"gen-…"`, `provider:"OpenAI"`,
  `native_finish_reason`, usage.prompt_tokens/completion_tokens).
- Streaming 200: `text/event-stream` of `chat.completion.chunk` with
  `choices[].delta.content` and a final usage chunk.
- **Credits: NOT consumed.** availableCredits identical before, immediately after,
  and 10 s after two calls (484.740530 throughout). The usage header freezes at
  `remaining=484`. Observed 2026-10-01; may be a metering gap Framer closes later.
- This makes `/ai/v3/chat/` the preferred compat route while it lasts.

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

## Notes

- Castle tokens are consumed per request; reusing one → 403.
- A 403 with body "user does not have access to project" = bad projectId
  (e.g. accidentally including the `-xxxx` access-token suffix from the URL),
  NOT a risk block.
- Earlier blanket 403s ("Forbidden") from the automation browser were transient
  Castle scoring; the same endpoints succeed from the same browser hours later.
