# Security Sweep — 2026-10-09 (implementation-bug hunt)

Systematic vulnerability research against Framer's own implementation
(not Castle). Method: controlled live testing from iris (residential IP,
multiple sessions/accounts) + decompiled client code review.

## Results matrix

| Attack class | Target | Result |
|---|---|---|
| SSRF | `/ai/v3/fetch/` | **DEAD — exemplary guard.** Blocks private IPs by DNS-resolution check, credentials-in-URL, non-HTTP(S) protocols, decimal/hex IP notation, and **re-validates after every redirect hop** (302→169.254.169.254 → 502 "not an external URL"). |
| Negative balance | all `/ai/*` routes on 0-credit workspace | **DEAD** — uniform 402 pre-check on every route, both categories, stream and non-stream, direct and passthrough. No debit-into-negative path. |
| IDOR | ledgers, projects, dashboards, AI-on-others'-projects | **DEAD** — 403 membership checks on all. Cross-account AI inference on another user's projectId: 403. |
| Phantom refund | refund turnIds with no charge | **DEAD** — `granted:false`. Refund only reverses real charges. |
| Refund race (same charge, 4 concurrent) | `/ai/v3/refund/` | **DB-protected** — one grant, others denied; one hit `500 "cannot record refund: ERROR: duplicate key value violates unique constraint"`. Unique key on the refund record. |
| Cross-user refund | refund account B's charge from account A's session | Envelope requires projectId+featureCategory (ownership-gated); project membership check blocks. (final full-envelope cross-test pending — result on iris) |
| Mass assignment | team creation with plan/credits/role/employee fields | **DEAD** — 11 crafted payloads all 201 but every extra field silently ignored; `license.type` stays `free`. |
| Workspace-grant race | 6 concurrent `POST /account/teams` on fresh account | **DEAD — transactional.** Exactly 2 grants total (500+500) regardless of concurrency. Winner of the second grant was racy (ws3 of 6) but the COUNT is invariant. |
| Metering integrity | stream → ledger | Metering is real-time via `x-framer-ai-usage: remaining=N` response header (integer floor). **The ledger endpoint `/web/teams/{t}/ai-credits` LAGS minutes** — ledger polling is unreliable for short-window tests; use the header. |

## Real findings (minor, but genuine)

1. **Unhandled DB error with schema disclosure** — concurrent refunds
   on one charge produce `500 "cannot record refund: ERROR: duplicate key
   value violates unique constraint \"r…\""`. Leaks the constraint/table
   naming. Low severity (info disclosure) but a genuine unhandled-error bug.

2. **Refund quota is weekly** — client code: "Reached weekly refund…"
   message. Settles the earlier quota-window question.

3. **Gmail plus-addressing works for signups** — `user+tag@gmail.com`
   accepted by Framer's signup; magic link delivered. Makes the account
   farm fully self-serve.

## Methodology notes

- `/ai/v3/refund/` requires the full standard envelope (projectId,
  featureCategory, featureName, step, sessionId, turnId, userPrompt,
  castleToken) — bare {turnId} → 400 "projectId is required", then
  "feature category is invalid".
- `/web/v2/dashboard/teams/{t}` requires a `collectionId` param.
- Project creation: `GET framer.com/projects/new?team=<spaceId>` →
  307 Location: /projects/<20-char-id> (must not follow redirects;
  spaceId = space.id NOT ownerId). Duplicate route:
  `POST /projects/new?duplicate=<id>&format=json&team=<teamId>`.
