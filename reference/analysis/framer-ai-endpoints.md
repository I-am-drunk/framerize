# Framer AI endpoints — live capture + static analysis

Sources: live network capture in the editor Agent panel (2026-10-01, controlled
account), static analysis of `chunk-MV5XJZF7.mjs` (transport + Gq envelope),
`chunk-IL7V6RK4.mjs` (providers), `chunk-5LKSDTQ5.mjs` (Castle),
`chunk-4B2T242G.mjs` (AI client), model registries in `chunk-DM5CYKH3.mjs` /
`chunk-WELZ4GZF.mjs`.

All on `https://api.framer.com`, all internal/undocumented.

## The gateway speaks every dialect (the big finding)

Framer's AI gateway fronts many upstreams. The web app creates AI-SDK providers
with placeholder baseURLs (`https://framer.ai-service.invalid/<name>`) and a custom
fetch that throws the placeholder away and POSTs to `{apiHost}{path}` instead:

```js
// chunk-IL7V6RK4.mjs (deobfuscated)
function makeFetch(path, meta) {
  return async (_ignoredUrl, init) => {
    const { apiHost, accessToken } = await getAiToken();      // Yq: mint/cache JWT
    return post(`${apiHost}${path}`, {
      headers: buildHeaders(init.headers, accessToken),       // qq: Bearer + ct + x-anthropic-beta
      body: await wrap(JSON.parse(init.body), meta),          // Gq envelope, below
      signal: init.signal,
    });
  };
}
```

| Dialect | Path | Upstream |
|---|---|---|
| OpenAI **Responses** | `/ai/v3/openai/responses/` | OpenAI direct |
| OpenAI Responses (alt) | `/ai/v3/responses/` | — |
| OpenAI **chat/completions** | `/ai/v3/chat/` | **OpenRouter** (OpenAI wire format) |
| **Anthropic Messages** | `/ai/v3/anthropic/messages/` | Anthropic direct |
| Anthropic Messages | `/ai/v3/messages/` | OpenRouter's Anthropic-format route |
| OpenAI-compatible | `/ai/v3/baseten/` | Baseten (Kimi K2.x/K3 etc.) |
| OpenAI-compatible | `/ai/v3/fireworks/` | Fireworks |
| OpenAI images | `/ai/v3/openai/images/generations/` | `gpt-image-2.5-flare` |

### The universal envelope (`Gq`)

Every dialect call's native payload is wrapped:

```json
{
  "projectId": "UtVRwMnJTYSkR8C46sQM",
  "featureCategory": "agents",
  "featureName": "agents",
  "step": "message",
  "sessionId": "z4FUSxAbP",
  "castleToken": "<single-shot>",
  "request": { /* the native payload, verbatim:
                  OpenAI Responses body, OpenAI chat/completions body,
                  or Anthropic Messages body — model, messages, stream, tools… */ }
}
```

Verified from the export map: `Gq` = the wrapper used by both the chat transport
(`Vde` → `/ai/v3/chat/`) and the proxy fetch (`g4`/`f4`/`Qa`).

**Consequence for the compat layer**: Claude Code's `POST /v1/messages` body can be
forwarded almost verbatim as `request` to `/ai/v3/anthropic/messages/`, and Codex's
Responses body to `/ai/v3/openai/responses/`. Framerize is a thin translating proxy,
not a re-implementation. `includeUsage: true` is set on the app's providers, so
streams carry usage; responses come back in the SAME dialect sent.

### The agent panel's own message call (special case)

The editor Agent chat does NOT send a `request` field — the server composes the
model call from `userPrompt` + `sessionId` (server-side session state, including
the pinned model). Captured verbatim:

```json
{"projectId":"…","featureCategory":"agents","featureName":"agents","step":"message",
 "sessionId":"z4FUSxAbP","stepIndex":0,"raindropEventId":"<uuid>","turnId":"LHXgk7pUf",
 "userPrompt":"Say hello in one word.","castleToken":"cNz8rPYH|JABF…"}
```

### Side tasks on `/ai/v3/chat/`

`step: "generate-session-title"` and `step: "compact-turn"` — header
`x-anthropic-beta: fine-grained-tool-streaming-2025-05-14,interleaved-thinking-2025-05-14`.
Models: cheap Gemini (`google/gemini-3-flash-preview`). Optional for clients; long
agent sessions may degrade without `compact-turn`.

## Auth & support endpoints

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/auth/ai/access-token` | session cookie | 600 s AI Bearer JWT |
| GET | `/auth/web/access-token` | session cookie | web API JWT (scopes 16383) |
| GET | `/web/users/me` | session cookie | profile + workspaces (`spaces[]`; credit team = `spaces[].ownerId`) |
| GET | `/web/teams/{teamId}/ai-credits` | web Bearer | credit ledger (funds/grants) |
| GET | `/web/projects/{projectId}/threads/?archived=false&branchId=main` | web Bearer | agent chat threads |
| GET | `/web/v2/dashboard/teams/{teamId}?collectionId=recent&orderBy=lastOpenedAt&limit=300` | cookie | dashboard project list for a team |
| GET | `/web/teams/{teamId}/join-link` | cookie | team invite link |
| POST | `framer.com/internal/activeusers` | cookie | presence (project ids array) |

Workspace creation ("Add workspace") did not open its modal in the automation
browser (React hover/click handlers inert under synthetic events) — the
create-team call remains UNCAPTURED; capture from a real browser.

## Status & headers policy

- 401 on `/ai/v3/*` → JWT expired; remint (cheap cookie GET) and retry once.
- 401 minting a JWT → session cookie dead; account needs re-login.
- 402 → workspace credits exhausted (`reason` in JSON body); rotate workspace.
- 403 → Castle risk refusal (generic `{"error":{"message":"Forbidden","ref":"…"}}`),
  transient; cool down. Also observed on `/web/teams/{id}/ai-credits` when the id
  is not a team the user can read ("User … is not allowed to perform this
  operation for team …") — different body, don't confuse with risk refusal.
- Success: `X-Framer-AI-Usage: remaining=<n>` (URLSearchParams) — free credit read
  on every inference response.

## Models

Two id conventions, verified live: `/ai/v3/chat/` (OpenRouter) takes PREFIXED
catalog ids (`openai/gpt-4.1`); `/ai/v3/openai/responses/` and
`/ai/v3/anthropic/messages/` take UNPREFIXED ids (`gpt-6-sol`, `claude-sonnet-4-5`).
The agent panel pins the model as server-side session state; direct `request`
calls set it per request.

## Metering (observed 2026-10-01 — see verified-captures for the ledger timeline)

- `/ai/v3/openai/responses/` and `/ai/v3/anthropic/messages/`: metered.
- `/ai/v3/chat/` (OpenRouter passthrough): UNCERTAIN. Tiny calls showed zero debit
  at 6-decimal precision, but a 7.7-credit debit landed during a window of
  long chat-route generations. Debits can lag ≥1 h (grant.updatedAt proves it).
  Treat as metered until an isolated retest says otherwise.
