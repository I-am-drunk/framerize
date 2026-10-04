# Framer AI credits — account vs workspace

Sources: [Framer help](https://www.framer.com/help/articles/how-ai-credits-and-agents-pricing-work/),
[pricing](https://www.framer.com/pricing),
[blog](https://www.framer.com/blog/ai-credits-simpler-plans-and-lower-prices/),
plus a live read of `GET /web/teams/{teamId}/ai-credits` on a controlled account.

## The answer: credits are workspace-based, not account-based

- The ledger is keyed by **teamId** (= workspace). URL shape: `/web/teams/{teamId}/ai-credits`.
- Credits are **shared across all editors and all projects** in the workspace.
- Allowances **stack across projects**: a workspace with two projects on the Basic
  site plan gets 2,000 credits/month.
- One user (account) can own or belong to many workspaces — each with its own ledger.
  This is what makes workspace rotation viable: an account with N free workspaces has
  N independent credit funds.

## Plan allowances (official, 2026)

| Plan | Included | Reset | Notes |
|---|---|---|---|
| Free | 500 credits | daily (capped below 1,000/month, no rollover) | "to try Agents"; pauses at limit |
| Basic ($10/mo) | 1,000 / month per site plan | monthly | top-ups anytime |
| Pro ($30/mo) | 3,000 / month per site plan | monthly | top-ups anytime |
| Enterprise | custom | — | "External agents" listed as PREVIEW on pricing |

Typical costs (Framer's own table, base model): landing page ~300, responsive pass
~150, large edit ~100, small edit ~50.

## Live evidence (free account, 2026-10-01)

Account structure from `GET /web/users/me` (cookie auth, no Bearer needed):

```json
{ "id": "1e7adbdb-…", "email": "…",
  "spaces": [
    {"id":"78a46bcf-…","scope":"user","isDefault":true,"ownerId":"1e7adbdb-…"},
    {"id":"141c9682-…","scope":"organization","displayName":"hghg's Workspace",
     "ownerId":"4713db54-…"}
  ] }
```

Credit probes on every candidate id:

| id | result |
|---|---|
| `4713db54-…` (workspace's owner TEAM) | **200** — 484.875 credits |
| `141c9682-…` (workspace SPACE id) | 403 "not allowed … for team …" |
| `78a46bcf-…` (personal space id) | 403 same |
| `1e7adbdb-…` (user id) | 403 same |

So: the credit key is the workspace's **owner team id** (`spaces[].ownerId`),
nothing else. The personal space has no credit ledger of its own.

The credited workspace's ledger:
```json
GET /web/teams/4713db54-ff6e-4698-863f-8fcbbd94d3d8/ai-credits?includeExpiredSince=…
{
  "teamId": "4713db54-…",
  "availableCredits": 484.875,
  "funds": [{
    "type": "free", "licenseId": null,
    "grants": [{
      "allocatedCredits": 500,
      "consumedCredits": 15.125,
      "remainingCredits": 484.875,
      "validAt": "2026-09-30T01:25:56Z",
      "expiresAt": "2026-10-30T01:25:56Z",
      "referenceId": "free-intro"
    }]
  }]
}
```

- Free workspaces get a one-time **500-credit intro fund** (`referenceId:
  free-intro`, 30-day validity) — observed; the daily-reset allowance is separate.
- Consumption is fractional (metered to ~1e-6) and model-dependent.
- `availableCredits` is the sum over non-expired funds.

### The "first N workspaces get credits" hypothesis

Unverified as of 2026-10-01: the "Add workspace" UI did not open a dialog in the
test environment (possible gating or a render failure), and the create-team
endpoint is not yet captured (`TeamCreationModal` receives `api.createTeam` from
its host). The claim that only the first two workspaces on an account ever receive
free credits is PLAUSIBLE (the `free-intro` referenceId reads as a per-account
acquisition grant, not per-workspace) but needs one live experiment: create
workspace #2/#3 in a clean browser, read `ai-credits` for each new owner team,
record `funds`. If new teams arrive with `funds: []`, rotation-by-creation is dead
and Framerize must rotate across ACCOUNTS instead. This experiment gates the
rotation design — run it before building `create_workspace`.

## Rotation semantics for Framerize

- **Exhaustion signals**: HTTP 402 on `/ai/v3/*`, and the `X-Framer-AI-Usage`
  response header (`remaining=<n>`, URLSearchParams format) on success — poll-free
  pre-flight: read the header off every inference response and cache it.
- **Workspace discovery** (TODO endpoint): the dashboard calls a teams/workspaces
  list; each team is a credit pool. Rotation = pick the same account's next teamId
  with `remaining > threshold`, create a fresh project in it, keep going.
- **Workspace creation**: free accounts can hold multiple workspaces ("Invite your
  team" flow creates one per team); limits observed in the UI: free workspaces ≤ 3
  editors, but workspace *count* per account needs a live probe.
- Unlike Notionize (workspace-per-exhaustion with `premium-feature-unavailable` in a
  200 stream), Framer fails with a hard 402 — simpler to detect.

## Workspace creation + the "which workspaces get free credits" experiment (2026-10-01)

**Endpoint (finally captured — statically from `chunk-TC7CTTY5.mjs`, then executed live):**

```
POST https://api.framer.com/account/teams
Headers: content-type: application/json, x-requested-by: Framer   (REQUIRED —
         constant "Framer"; missing → 403 "Missing x-requested-by header")
Body:    { "teamName": "Workspace Two" }        (field is teamName, not name)
→ 201 { teamId, teamName, license: { type: "free", featureFlags… } }
```

(The "Add workspace" UI stays inert under automation — React state modal — but the
API itself is trivially callable. Related: `PUT /account/teams/{teamId}`,
`POST /web/v3/teams/{teamId}/subscription/sync/preview`.)

**Experiment: does a NEW workspace on an existing account get the 500-credit
free-intro grant?** Created "Workspace Two" (`fd5029ba-f67b-4d4f-b6a8-e879e9e2b80b`)
on the test account (which already had workspace #1 "hghg's Workspace" with the
grant, plus the default personal space):

```
GET /web/teams/fd5029ba…/ai-credits
→ 200 { "availableCredits": 0, "funds": [] }     (still 0 after 2+ min — no async grant)
```

**A newly created workspace gets ZERO AI credits.** This kills the "rotate across
workspaces on one account" design — rotation must be across ACCOUNTS. The user's
"first two workspaces get credits, never again" claim is half-confirmed: later
workspaces get nothing. Whether the free-intro grant lands on the first one or
the first two workspaces at signup is indistinguishable on this account (the
personal space has no team ledger); a fresh-account experiment would settle it.

## Per-project grants: the measurement, and what is still missing (2026-10-04)

**Observed on a freshly added account** (`user 991c1d1d`, team `8cd3b529`):

```
availableCredits: 1000.0    funds: 1
```

1000 = 2 x 500, in ONE fund. That matters because the earlier code counted
*funds* and would have read this as a single 1000-credit fund. The ledger nests
`funds[].grants[]` (see the JSON above, line ~61), so the correct unit for "how
many 500-credit allocations does this account hold?" is the GRANT count.

`CreditSnapshot` now carries:
- `grants` — total grants across all funds
- `allocated` — sum of `allocatedCredits` (lifetime entitlement)
- `grant_count()` — `grants`, falling back to `allocated / 500`

This corroborates line 11-13 ("allowances stack across projects") and the
operator's report: a new Framer account can hold up to two credited projects at
500 each, and nothing after that.

### Blocked: no project-creation endpoint has been captured

Auto-creating projects to maximise grants needs `POST <create project>`, which
is **not** in the captured bundles. Searched
`reference/decompiled/app/full-chunk-set/` for `createProject`, `newProject`,
and every `le.post`/`.post("/web/...project...")` call site. What exists:

| method | path | purpose |
|---|---|---|
| GET | `/web/v3/projects/search/` | project search |
| GET | `/web/v2/dashboard/teams/{teamId}` | dashboard project list |
| POST | `/web/v4/projects/{id}/subscription/move` | move a subscription |
| POST | `/web/v1/sites/hostnames/custom/projects/{id}/validate` | custom hostname |
| POST | `/account/teams` | create WORKSPACE (captured, works) |

`newProject` appears only as a UI symbol, with no fetch beside it — the create
call is built elsewhere (same shape as the `TeamCreationModal` problem that kept
`createTeam` uncaptured until 2026-10-01).

**Next step to unblock, in order of cost:**
1. Capture it from a real browser: devtools Network, create a project, copy the
   request (method, path, body, required headers — note `x-requested-by: Framer`
   is mandatory on `/account/*`).
2. Then: on admission, read `grant_count()`; while it is below the per-account
   cap, create a project and re-read the ledger to confirm a grant landed.
3. Stop on the first creation that does NOT add a grant — that is the real cap,
   and it must be discovered rather than hardcoded at 2.

Writing the creation call against a guessed endpoint would burn the account's
one-time grants on failed attempts, so it waits for the capture.
