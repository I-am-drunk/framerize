# Framerize as a Claude Code / Codex gateway

Researched against Anthropic's official gateway protocol spec
(`code.claude.com/docs/en/llm-gateway-protocol`, fetched 2026-10-01) and
OpenAI's Codex config reference (`developers.openai.com/codex/config-reference`),
plus credible third-party implementations (ccode's local proxy, LiteLLM's
gateway, claude-code-router). Framerize IS the gateway — no extra proxy layer.

## Claude Code

```bash
export ANTHROPIC_BASE_URL=http://127.0.0.1:8787/anthropic   # bare path = FREE (unmetered chain)
export ANTHROPIC_AUTH_TOKEN=local                           # any non-empty value
claude
```

- Setting `ANTHROPIC_AUTH_TOKEN` replaces subscription login for the session
  (official behavior) — exactly what we want: Claude Code treats the daemon as
  the provider.
- **Convention: the bare routes are free.** The metered streaming passthroughs
  live under `/paid/`: `ANTHROPIC_BASE_URL=http://127.0.0.1:8787/paid/anthropic`.
  Claude Code cannot send custom headers, so the free path is the default at
  the normal URL; `x-framerize-mode: paid` remains as a header override.

### Protocol conformance checklist (from the official spec)

| requirement | framerize status |
|---|---|
| `POST /v1/messages` | ✓ `/anthropic/v1/messages` (free by default; `/paid/` twin is metered) |
| `POST /v1/messages/count_tokens` (optional) | ✓ char-based estimate (same fallback the client uses) |
| `HEAD /api/hello` warming probe | ✓ answered 200 at `/api/hello` |
| streamed responses, full event sequence, ordered | ✓ synthesized SSE (free path) / raw passthrough (metered) |
| keep-alive `ping` events during silent gaps | TODO: emit pings while a chain chunk is in flight |
| forward `anthropic-beta` / `anthropic-version` | n/a upstream is Framer, not Anthropic — the daemon consumes them |
| model ids: Anthropic form (`claude-opus-4-8`) | mapped by `to_openrouter_id` → OpenRouter catalog |

### Model mapping

Claude Code sends Anthropic ids. The daemon normalizes to OpenRouter catalog
ids (`claude-opus-5-5` → `anthropic/claude-opus-5.5`). Unknown/aliased ids pass
through and error visibly from upstream rather than being silently remapped.
Pin models client-side with the documented env vars
(`ANTHROPIC_DEFAULT_OPUS_MODEL`, `ANTHROPIC_DEFAULT_SONNET_MODEL`,
`ANTHROPIC_DEFAULT_HAIKU_MODEL`) to any catalog id the account can serve —
free-tier verified: `claude-sonnet-4-5`, `claude-opus-5-5`, `gpt-4.1`,
`gemini-2.5-flash` (NOT `claude-fable-5.1` — `paid_plan_only_model`; NOT
`gemini-3-flash-preview` — 403 on free tier).

## Codex

`~/.codex/config.toml`:

```toml
model = "openai/gpt-4.1"        # any OpenRouter catalog id
model_provider = "framerize"

[model_providers.framerize]
name = "Framerize"
base_url = "http://127.0.0.1:8787/openai/v1"   # free by default; /paid/openai/v1 = metered
env_key = "FRAMERIZE_LOCAL"                          # any non-empty string
wire_api = "responses"          # the ONLY supported value since Feb 2026
```

- `wire_api = "chat"` was REMOVED from Codex in Feb 2026 — Responses format is
  mandatory; framerize's `/openai/v1/responses` serves it.
- Codex's own SSE watchdog defaults to 300s idle (`stream_idle_timeout_ms`) —
  far above any framerize window.

## The never-fail ladder (v3, implemented)

For 24/7 automation the rule is "a long request can never cause an issue":

1. **free non-streaming** on `/ai/v3/messages/` (~100s upstream window —
   measured: 44.9s call completed, 109s killed; chat route's window is 21.5s);
   adaptive chunk sizing measures live tokens/sec and budgets ~45s/chunk.
2. **halving retry** on a window death (distinguished from a Castle refusal by
   its >8s silence — Castle refuses instantly).
3. **metered streaming for the failing chunk only** (`metered_fallback`,
   default ON): SSE flows bytes so it cannot idle-die. The chain then resumes
   free. `ChainResult.metered_chunks` reports any credit spend so automations
   can alert on it.

Streaming requests from clients (Claude Code always streams) get synthesized
SSE from the chain: full ordered event sequence, with per-chunk deltas.

## Streaming contract (v0.6)

Compat endpoints stream INCREMENTALLY: the SSE connection opens with
message_start / role / response.created in the first milliseconds, chunk
deltas arrive as each chain chunk completes, and heartbeat pings
(anthropic `ping` events / SSE comments) fire every 8s of upstream silence.
A chain's total runtime no longer matters to client timeouts — no socket
ever idles. (The v0.5 buffered-then-synthesize approach produced exactly
that failure: clients read-timed-out mid-chain.)

## Losslessness contract (v0.5 architecture)

The free path is a **block-faithful relay**, not a text proxy:

- **Anthropic clients**: the request passes to the upstream **verbatim** (only
  `model` is normalized, `stream` forced off per chunk, `max_tokens` set per
  chunk). `tools`, `tool_choice`, `system` (string or block arrays with
  cache_control), `temperature`, `top_p`, `top_k`, `stop_sequences`,
  `metadata`, `thinking` all flow upstream untouched. Response content blocks
  re-emit untouched (including upstream extras like `citations`).
- **OpenAI chat / responses clients**: every field with an Anthropic
  equivalent is mapped — `tools` ↔ anthropic tools, `tool_calls` ↔ `tool_use`,
  tool results ↔ `tool_result` user blocks, `instructions` ↔ system, images
  (url + base64) ↔ image blocks, `user` ↔ `metadata.user_id`. Documented drops
  (no Anthropic equivalent): penalties, logprobs, seed, n>1, reasoning blobs.
- **Chaining is block-aware**: text blocks merge seamlessly across chunks;
  `tool_use` blocks keep ids/names/inputs; a chunk cut mid-`tool_use` retries
  with a doubled budget instead of chaining through a half-written call;
  `stop_reason` and usage pass through exactly.
- **Streaming**: synthesized SSE emits the complete dialect event grammar —
  Anthropic (`content_block_start` typed blocks, `input_json_delta`,
  `message_delta` with stop_reason), chat (`tool_calls` deltas with indices,
  usage chunk behind `stream_options.include_usage`), responses
  (`output_item.added/done`, `function_call_arguments.delta/done`,
  `response.completed` with the full object).
- Verified live on iris (2026-10-03): tool_use via stream + JSON, multi-turn
  tool_result loop, Codex-shape function_call SSE, chat tool_calls, 1.5k-token
  chained generation — all `metered_chunks=0`.
