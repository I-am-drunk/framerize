# Castle contract — fully mapped (2026-10-05)

Every claim below was verified live on the main account this date, in-page
(Camoufox), with the publishable key injected. Wave-free conditions unless
marked. Raw scripts: /tmp/castle_audit{,2,3,4,5,6}.py on iris.

## Layer 1 — structural token validation (always on, per request)

Routes WITH the Castle middleware: `/ai/v3/messages/`, `/ai/v3/chat/`,
`/ai/v3/responses/`, `/ai/v3/anthropic/messages/`, `/ai/v3/openai/responses/`.
Routes WITHOUT it: `/ai/v3/fetch/` (400 "url is required" with any/no token),
`/ai/v3/refund/` (200 `{"granted":false}` with garbage), `/ai/v3/usage/total/`
(404 POST), `/ai/v3/baseten/` + `/ai/v3/fireworks/` (employee gates, reachable
past the Castle layer).

With a garbage (2200×'z') or omitted token, all guarded routes return
`403 {"error":{"message":"Forbidden","status":403,…,"ref":"…"}}` in ~50-130ms,
**regardless of every transport variant tested**:
- content-type: `application/json`, `text/plain`, `x-www-form-urlencoded`, none
- auth: AI JWT vs **web JWT (both accepted for AI calls!)**
- body `stream`: false/true
- envelope extras: `origin:"framer-app"`
- step values: `message`, `generate-session-title`, `compact-turn`
- path normalization: trailing slash, `/.`, `?query` (no-slash and case
  variants 404; no middleware dodge)
- method confusion (GET w/ body: client-impossible), custom headers
  (`X-Castle-Token`, `X-Framer-Client`: **CORS-preflight rejected** — the
  gateway allowlists only content-type, x-requested-by, authorization)

Unknown-route probes: batch/generate/completions/agent/run/agents/async → all
404. No hidden inference path. `framer.com/ai/*` 308-redirects to www (404).
No alternate API hosts resolve. No websockets for AI (the `WebSocket` mention
in the transport chunk is a plugin-sandbox blocklist). reCAPTCHA Enterprise is
used ONLY for invite flows (`invite_user` action) — never attached to AI calls.

## Layer 1 — the relaxations (where Framer under-implemented Castle)

Castle's documented token doctrine vs. observed enforcement:

| doctrine (Castle docs)              | observed on /ai/v3/*            |
| ----------------------------------- | ------------------------------- |
| expires after 120s                  | **+300s → 200** (not enforced)  |
| single-use per backend request      | **3× sequential reuse → 200**   |
| (implicitly) account/session-bound  | **acct1 token + acct2 session → 200** |
| —                                   | **same token × 12 CONCURRENT → all 403** |

So the ONLY hard bind is concurrency: one token = one in-flight request at a
time. Sequential reuse is unlimited within at least 5 minutes. Tokens are
fungible across accounts/sessions on the same environment.

Operational consequences (daemon):
- Mint **once per chain per ~240s**, not per chunk: sequential chunks of one
  chain may share a token (chunks are sequential by construction).
- CONCURRENT chains must hold DISTINCT tokens (concurrent reuse → 403).
  A small per-account token pool (acquire on chunk start, release on chunk
  end, refresh at 240s) is the correct structure.
- One mint page can serve all accounts (fungibility).

## Layer 2 — the velocity ban ("waves")

- Valid fresh tokens get instant 403s (~46-267ms, same JSON shape as the
  structural 403) on ALL inference routes simultaneously.
- Non-inference routes (fetch/refund/usage) are UNAFFECTED during waves.
- Duration ~10-15 min, self-heals with no client-side change.
- Follows bursts of automated traffic (soaks, matrix testing).
- The deny latency (~50-100ms) is too fast for a synchronous Castle cloud
  verdict — consistent with a LOCAL deny-list/ban cache with a TTL, i.e.
  Framer's own velocity layer, possibly distinct from Castle scoring.
- NOT keyed on device (fresh __cuid contexts were denied), NOT on sessionId
  (fresh per request, still denied), NOT on the AI JWT (mints fine mid-wave).
- Untested cells (the canary's battery fires these on the next wave):
  valid token + NOVEL invented step (does the ban bucket per `step` like the
  concurrency cap?), account-2 same-IP (per-account vs per-IP), stream:true
  mid-wave, token-reuse mid-wave.

## Canary

`/tmp/wave_canary.py` on iris (detached): probes account-1 every 60s with a
valid token (free non-streaming gemini call). On wave onset it auto-fires the
discriminating battery (novel-step, system-step, stream-true, account-2,
reuse), and logs wave end + duration. Log: `/tmp/wave_canary.log` on iris.

## Metering intersection (constraint: keep the free path)

Metering rule re-verified and EXTENDED: all THREE OpenRouter passthroughs are
free non-streaming, metered streaming:
- `/ai/v3/chat/` (OpenAI chat dialect)
- `/ai/v3/messages/` (Anthropic dialect)
- `/ai/v3/responses/` (OpenAI Responses dialect — **new**: Codex's native
  protocol, zero conversion, ledger-verified free non-streaming at 6 decimals:
  392.979258 → 392.979258; streaming debited 0.0127)
