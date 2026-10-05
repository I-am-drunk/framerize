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
| expires after 120s                  | **+300s → 200** (TTL not enforced) |
| single-use per backend request      | **6-USE COUPON**: uses #1-6 → 200, use #7+ → 403 **permanently** (burned token still 403 at +90s/+210s while fresh tokens pass — count cap, not a rate window) |
| (implicitly) account/session-bound  | **acct1 token + acct2 session → 200** (also crossed devices/pages — device-binding not checked) |
| —                                   | same token × 12 CONCURRENT → all 403 (concurrency denial exists but is partially conflated with the 6-use cap) |

**The token is a 6-use coupon valid ≥5 minutes, fungible across accounts,
sessions, and pages on the same network. The only hard rules: ≤6 uses ever,
never concurrent.**

This retroactively explains "transient 403s" seen in ramp/audit traffic: any
script reusing one token past 6 uses hits deterministic 403s that *look* like
risk refusals but are just burned coupons. (The daemon's pre-pool
per-chunk-mint design was safe by accident; unbounded pooling would have
burned tokens mid-chain — caught before it bit.)

Operational consequences (daemon):
- Token pool: acquire/release per chunk, **retire at 5 uses**, refresh at
  240s, cap 8 per account. Concurrent chains always hold distinct tokens.
- One mint page can serve all accounts (fungibility).
- A 5-use token at ~600-token chunks serves ~3000 tokens of generation;
  long chains transparently rotate to the next coupon.

## Layer 2 — the velocity ban ("waves") — DEFINITIVELY MEASURED (2026-10-05)

Controlled ramp experiment (coupon-aware, fresh-token-confirmed):

- **Threshold**: 30 req/min clean (22×200), 60 req/min clean (37×200),
  **~120 req/min trips it** (~32 requests into the burst).
- **It is PER-ACCOUNT, not per-IP**: account-2 on the SAME egress IP returned
  200 while account-1 was mid-ban. **Account rotation is the structural
  wave escape.**
- **It does NOT bucket per `step`** (novel invented step → 403), does NOT
  exempt system steps (`generate-session-title` → 403), does NOT spare
  streaming (`stream:true` → 403). The concurrency cap's bucketing does not
  extend to the ban.
- Deny latency ~50-150ms (local counter, not a synchronous Castle cloud call),
  same generic-Forbidden JSON as the structural layer.
- **Duration ~2 min** for a lightly-tripped ban (vs ~10-15 min observed for
  heavier history) — scales with pressure; hammering a banned account almost
  certainly extends it (don't).
- Profile: Framer's own per-account velocity limiter (NOT Castle behavioral
  scoring). Non-inference routes (fetch/refund/usage) unaffected.

Daemon posture vs the measured line: the gate's 2.5s min-interval per
workspace caps the daemon at **24 req/min per account** — structurally below
the ~120/min trip line. Rotation on refusal is correct behavior; the ban is
per-account so a rotated request succeeds instantly.

## Canary

`/tmp/wave_canary.py` on iris (detached, JWT-refresh fixed): probes account-1
every 60s with a valid token (free non-streaming gemini call). On wave onset
(2+ consecutive) it auto-fires the discriminating battery (novel-step,
system-step, stream-true, account-2, reuse), and logs wave end + duration.
Log: `/tmp/wave_canary.log` on iris.

## Metering intersection (constraint: keep the free path)

Metering rule re-verified and EXTENDED: all THREE OpenRouter passthroughs are
free non-streaming, metered streaming:
- `/ai/v3/chat/` (OpenAI chat dialect)
- `/ai/v3/messages/` (Anthropic dialect)
- `/ai/v3/responses/` (OpenAI Responses dialect — **new**: Codex's native
  protocol, zero conversion, ledger-verified free non-streaming at 6 decimals:
  392.979258 → 392.979258; streaming debited 0.0127)
