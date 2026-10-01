# Claude Code adapter (Anthropic `/v1/messages`)

Claude Code points at a base URL and speaks Anthropic Messages SSE. Set
`ANTHROPIC_BASE_URL` to the Framerize daemon and it works unchanged.

Mapping (full detail in `../../reference/analysis/compat-mapping.md`):

- `system` → prepended to the first `userPrompt` of a Framer agent session.
- last `messages[].content` → `userPrompt`; history is server-side in Framer,
  keyed by the sessionId Framerize mapped to this conversation.
- stream → upstream `/ai/v3/openai/responses/` SSE events
  (`response.output_text.delta`, `response.completed`) are re-wrapped as
  `message_start` / `content_block_delta` / `message_stop`.
- `model` → advisory until the session-model-pinning endpoint is mapped
  (see compat-mapping.md "Known gaps").

Point Claude Code at it:

```sh
export ANTHROPIC_BASE_URL=http://localhost:8787/anthropic
export ANTHROPIC_AUTH_TOKEN=<framerize-local-token>
```
