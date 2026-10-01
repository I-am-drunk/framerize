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
