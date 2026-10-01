# Codex adapter (OpenAI `/v1/responses`)

Codex speaks the OpenAI Responses API — the same shape Framer proxies upstream,
so this adapter is nearly a passthrough.

- `instructions` + `input` → flattened to `userPrompt`.
- `previous_response_id` / conversation id → mapped Framer `sessionId`.
- `stream: true` → upstream SSE forwarded with light event translation.

```sh
export OPENAI_BASE_URL=http://localhost:8787/openai
export OPENAI_API_KEY=<framerize-local-token>
```

See `../../reference/analysis/compat-mapping.md` for the full field mapping and
the v1 limitations (no client tool-calling yet).
