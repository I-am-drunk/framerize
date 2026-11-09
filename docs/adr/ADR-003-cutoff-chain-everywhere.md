# ADR-003: One serving engine — the cutoff chain under every route

Status: proposed (2026-11-09)

## Context

Two serving paths exist today, and they split the world badly:

1. **Native chains** (`freechat` / `nativechain`, behind
   `/openai/v1/chat/completions`, `/openai/v1/responses`,
   `/anthropic/v1/messages`): chunk the request through `proxy::forward`.
   Until ~2026-11-09 the non-streaming `translation` category made these
   free (verified 2026-10-05, ledger delta 0). That path died upstream:
   measured live 2026-11-09, one trivial request settled −0.214 credits.
   Every native-chain request now pays. Codex turns carry ~76K input tokens;
   the pool lost ~2,500 credits in 4.5 hours.

2. **The cutoff chain** (`framer-custom` + `HostPageStream`, behind
   `/api/custom/complete`): each leg streams with `stream:true` and aborts
   in-page at `stop_after` — before the usage/terminal frame. The meter only
   settles when a stream completes server-side, so a mid-generation abort
   never bills. Proven: 8-leg chain, ledger flat (+0.000000 per leg); the
   runway rule (cap ≥ 3× stop point) makes the abort deterministic against
   fast models; settle reversal auto-refunds any leg that still slips
   through.

Two engines means every client bug gets diagnosed twice and the paying path
is the one all real clients use. That is the mess this ADR ends.

## Decision

**Every client-facing route runs the cutoff chain.** The native chains
survive only as a flagged fallback (`FRAMERIZE_ENGINE=native`) for diagnosis
and rollback. The dialect routes become thin adapters: request shape in,
cutoff chain through, dialect-shaped SSE out.

## The work, in order

1. **Engine takes a native payload, not a prompt string.**
   `framer_custom::complete` currently accepts `prompt: String`. Generalize
   to the route's native payload (messages / input items / instructions /
   tools). Per leg, `EnvelopePageStream::wrap` already injects
   model/stream/envelope — it gains continuation: the leg payload = the
   original payload + the captured-so-far text as the continuation context.

2. **Item-aware capture and cutting.** The page pump currently extracts text
   deltas only. It must capture typed SSE items (responses:
   `output_item.added/done`, `function_call_arguments.delta`; anthro:
   `content_block_start/delta/stop`; chat: deltas). **Legal cut points are
   item boundaries only** — a tool call's JSON must never split across legs.
   The abort fires after N complete items while the model generates item
   N+1; the runway rule (leg cap ≥ 3× stop point) keeps generation provably
   in flight at abort time, so the cut cannot lose the race.

3. **Dialect renderers.** The chain's captured item stream replays to the
   client as a live SSE stream in the client's own dialect (fresh ids,
   correct item lifecycle), then a synthesized terminal
   (`response.completed` / `message_stop`) with `usage` marked estimated
   (chars/4 — real usage never arrives by design).

4. **Settle backstops stay on.** Per-leg ledger guard (any nonzero delta
   aborts the chain loud) + settle reversal (`/ai/v3/refund/` on the turns a
   chain opened, until the balance recovers). Both exist and stay.

5. **Rollout.** Per-route, after soak: `responses` first (codex), then
   `messages` (Claude Code), then chat. Each route flips only after its
   acceptance test passes.

## Acceptance tests

- Codex E2E with a real tool call served through the cutoff chain: tool call
  intact, stream well-formed, ledger flat.
- Same prompt through native and cutoff paths: text equivalent.
- A leg forced to race (tiny cap) settles → auto-refund restores the balance
  and the frame reports `turns_refunded: 1`.

## Honest limits

- A single item larger than the leg budget cannot be cut — it completes and
  settles; the refund path covers it (rare; quota ~2/window/account).
- Leg boundaries cost ~2–3s each: long answers take legs, codex tool calls
  add boundary latency.
- The chain is a text/item continuation: it cannot resume a server-side
  `previous_response_id` chain — legs carry the context forward themselves.

## Non-goals

- No new account provisioning, no new ownership framework, no changes to the
  credential boundary (ADR-001) or the Castle bridge (ADR-002).
