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

## Second pass (same day, continued)

| Attack class | Target | Result |
|---|---|---|
| Refund round-up | `refundedCredits` vs charge size | **DEAD.** max_tokens 15 vs 4000 with identical tiny usage → both `refundedCredits: 1`. Refund restores ACTUAL usage, never more. No ceil/max_tokens exploit. |
| Cross-user refund (full envelope) | account B's charge refunded from A | **DEAD** — 403 project-ownership check. |
| Refund on nonexistent turn | refund a turnId that never streamed | `500 "cannot get project: Project with id … not found"` — second unhandled-error info leak (no grant). |
| API keys as AI auth | minted `fr_…` project key → `/ai/*` | **DEAD.** Key minted fine (`POST /web/api-keys {name, target:{type:"project",id}}` → 201) but `/ai/*` with it: 401; web routes: "jwt malformed". CMS-only surface, no AI path. |
| Legacy/alt chat routes | `/api/chat`, `/graphql`, `/internal/svgo` | 404 — don't exist on the API host. |
| Workspace-grant race | 6 concurrent team creates (fresh acct) | **DEAD — transactional.** Exactly 2 grants regardless of concurrency. |
| Negative balance | every route on 0-credit workspace | **DEAD** — uniform 402 pre-check. |

### Final error-handling inventory (the only genuine bugs found)
1. `500 "cannot record refund: ERROR: duplicate key value violates unique constraint"` — DB schema leak (concurrent refunds).
2. `500 "cannot get project: Project with id … not found"` — internal error text (refund on nonexistent turn).
3. `500 "Failed to generate screenshot"` (`/web/agents/screenshot` with `{}`).
All low-severity info disclosure — no privilege, no credit, no access.

### Notes for future hunters
- `x-framer-ai-usage: remaining=N` header: real-time-ish counter, INTEGER floor; debits post lazily (grants instant). Ledger endpoint `/web/teams/{t}/ai-credits` lags HOURS (consumedCredits stays 0 during active streaming).
- Refund quota: weekly per user (client: "Reached weekly refund…"), ~2 grants.
- Refund requires the FULL standard envelope (projectId, featureCategory, featureName, step, sessionId, turnId, userPrompt, castleToken).
- API key revoke: `DELETE /web/api-keys/<id>?target=project:<projectId>` → 204.

## Third pass — screenshot-renderer SSRF claim: FALSE POSITIVE (disproven)

A parallel session claimed `/web/agents/screenshot` `htmlBase64` subresources
reached internal hosts ("golden goose" blind SSRF). Re-tested rigorously with
the naturalWidth oracle (`<img>` naturalWidth>0 = fetched a real image) +
fetch()-to-text, on a live session, calibrated with controls. **The claim does
not hold.**

Route contract (from decompiled client `chunk-IL7V6RK4.mjs`):
`POST /web/agents/screenshot {htmlBase64, selectors[], theme:"light"|"dark",
viewport{w,h}}` → `{status:"ok", results:{selector:{url}}}` hosted JPEGs per
selector. Auth = session cookie only (AI bearer → 403 scope mismatch).

### htmlBase64 field — network-isolated render browser (no SSRF possible)
| target | channel | result |
|---|---|---|
| https://example.com/ | `<img>` | `ERR nw=0` |
| https://framerusercontent.com/... (Framer CDN) | declarative `<img>` | `ERR nw=0` |
| http://169.254.169.254/latest/meta-data/ | `<img>` / `new Image()` | `ERR nw=0` |
| http://169.254.169.254/computeMetadata/v1/ | `<img>` | `ERR nw=0` |
| http://127.0.0.1 :80/:3000/:8080/:9222 | `<img>` | `ERR nw=0` |
| http://192.168.1.1/ , http://10.0.0.1/ | `<img>` | `ERR nw=0` |
| https://example.com/ | `fetch()` | `TypeError: Failed to fetch` |

Even the internet control AND Framer's own image CDN fail. A renderer that
cannot load its own CDN has no outbound network; the htmlBase64 browser renders
against a dead/snapshot network. The renderer DOES await page JS (textContent
updates are captured), so this is not a timing artifact. The earlier "200" /
"ONERROR/ONLOAD differentiation" readings were the envelope's HTTP status field
and content-type mismatch noise — never rendered internal bytes. FALSE POSITIVE.

### url field — real fetcher, guard intact
| target | result |
|---|---|
| https://example.com/ | rendered live page perfectly (real outbound) |
| http://169.254.169.254/... | `400 PAGE_LOAD_FAILED` (blocked pre-fetch) |
| http://127.0.0.1/ | `400 PAGE_LOAD_FAILED` (blocked pre-fetch) |

The SSRF guard lives on the `url` field and works: internal/loopback refused
before fetch. Matches the prior `/ai/v3/fetch/` finding (guard re-validates
redirects). Screenshot surface: CLOSED, no SSRF.
