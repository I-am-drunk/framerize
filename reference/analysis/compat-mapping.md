# Compat mapping — Anthropic & OpenAI API routes on top of Framer

Goal: Claude Code and Codex talk to Framerize as if it were Anthropic / OpenAI.

## Key fact (from chunk-IL7V6RK4.mjs + Gq in chunk-MV5XJZF7.mjs)

Framer's gateway ALREADY speaks all three dialects natively:

| Client speaks | Framerize forwards to | Upstream |
|---|---|---|
| Anthropic `/v1/messages` | `POST /ai/v3/anthropic/messages/` | Anthropic direct |
| Anthropic via OpenRouter | `POST /ai/v3/messages/` | OpenRouter |
| OpenAI `/v1/responses` (Codex) | `POST /ai/v3/openai/responses/` | OpenAI direct |
| OpenAI `/v1/chat/completions` | `POST /ai/v3/chat/` | OpenRouter |

So Framerize is a **thin translating proxy**, not a re-implementation:

```
outbound body = Gq envelope = {
  projectId, featureCategory, featureName, step, sessionId, castleToken,
  request: <the client's native payload, nearly verbatim>
}
response    = same dialect the client sent (SSE if stream:true)
```

## Anthropic route (Claude Code → /anthropic/v1/messages)

- Forward `model`, `messages`, `system`, `max_tokens`, `stream`, `tools` inside
  `request` unchanged. The app itself sends Anthropic tool definitions upstream
  (fine-grained-tool-streaming beta header observed), so tool-calling SHOULD
  survive — verify with a clean capture.
- Set headers like the app: `x-anthropic-beta:
  fine-grained-tool-streaming-2025-05-14,interleaved-thinking-2025-05-14` when the
  client declares compatible betas.
- Streaming: Anthropic SSE in, Anthropic SSE out — translate nothing, just re-emit.

## OpenAI routes (Codex → /openai/v1/responses or /v1/chat/completions)

- Responses API: forward to `/ai/v3/openai/responses/` nearly verbatim.
- chat/completions: forward to `/ai/v3/chat/` (OpenRouter format — the app sets
  `includeUsage`, so `stream_options: {include_usage: true}` behavior exists).
- Reasoning effort mapping exists upstream (the app maps "enabled"/"high" per
  provider); pass `reasoning` through and let the gateway handle it.

## Per-request requirements (both routes)

1. Resolve account → session cookie → mint/cache AI JWT (`/auth/ai/access-token`).
2. Fresh Castle token from that account's webview (single-shot).
3. `projectId` = the account's current project (its workspace pays the credits).
4. `sessionId`: map the client's conversation id (Codex `previous_response_id` /
   prompt-cache key; Claude Code sends none — use a header or hash) to a stable
   Framer sessionId so server-side history/compaction works.
5. On 402 → rotate workspace (new projectId) and retry once; on 403 → cool down.

## TODO / verify with a clean 200 capture

- Exact `step`/`featureCategory` values the app uses for NON-agent proxy calls
  (the agent uses "agents"/"agents"/"message"; the proxy call-sites pass meta —
  capture one Anthropic-route call from the UI to copy its meta verbatim).
- Whether `/ai/v3/anthropic/messages/` accepts arbitrary models or only Framer's
  registry ids.
- The session-model-pinning endpoint (UI model picker → server state).

## VERIFIED 2026-10-01 (see verified-captures-2026-10-01.md)

- All three dialect routes work from a logged-in session with JWT + Castle token.
- `/ai/v3/chat/` (OpenRouter): tiny calls showed zero ledger debit, but long
  generations coincided with a 7.7-credit drop and debits lag ≥1 h — metering
  UNCERTAIN, treat as metered.
- `/ai/v3/anthropic/messages/` accepts `claude-sonnet-4-5` and returns native
  Messages JSON. Metered.
- `/ai/v3/openai/responses/` requires `request` and unprefixed model ids. Metered.
