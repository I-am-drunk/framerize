# Framerize

Framerize is a browserless Node server written in TypeScript. Framerize Remote
is a small local instance that forwards inference to that server. Castle
Library belongs on the server. This branch is work in progress, not a release.
Castle's encoder is vendored in `framerize-server/src/castle.ts`. Normal builds
use that local copy and never fetch the Castle repository. Update it only with
`scripts/vendor-castle-encoder.mjs` from a local checkout, and keep frozen
Castle work frozen unless its owner authorizes a change.

## Keep the design simple SIMPLENESS IS THE BIGGEST THING. CHANNEL YAGNI

- Understand the user's problem before choosing a mechanism. Reuse working
  code when it fits; explain why a new layer or dependency is necessary.
- Keep provider accounts, Castle and upstream routing out of Remote and core.
  Share client-facing HTTP behavior rather than adding another listener.
- Keep protocol compatibility distinct from provider-specific behavior.
  Preserve the selected model, options and streaming mode; do not silently fall back.
- Prefer explicit TypeScript types at boundaries and inferred types internally.
  Narrow untrusted values from `unknown`; do not hide uncertainty with `any`.
- Planning and review do not authorize implementation. Preserve unrelated work.

## Do not idle while work is available

This rule applies to every agent session and subagent.

- Waiting on another thread, subagent, review or CI is prohibited while
  independent, authorized work remains. Take the next unblocked task instead.
- Use completion notifications for delegated work and the owning session's
  existing PR watcher for review and CI results. Do not create duplicate watchers.
- Do not use blocking thread-wait tools, sleep loops or repeated status polls
  to wait for those results. Check status when a notification arrives or when
  the next action needs a fresh result.
- Before yielding for a dependency, check the plan for useful independent work.
  If none remains, record the exact blocker, keep the appropriate notifications
  active and end the turn. Resume when the result arrives.
- Do not invent busywork, repeat unchanged tests or reviews, bypass merge gates,
  or modify another owner's work to avoid waiting. Keep ownership boundaries.

## Learn from legacy without recreating it

- Legacy code is a reference, not the product requirements or a completion checklist.
- The old web app, relay and systemcmd are excluded. This is not the full exclusion list.
- Classify proposed behavior as keep, rework, remove, add or undecided. Do not
  turn an inventory difference into a feature requirement without a user need.
- New requirements can replace old behavior. Preserve the wanted result, not
  a mechanism that caused bugs. Add a regression for a reproduced failure.
- Do not publish overall completion percentages from an unsettled feature list.

## Cover the affected paths

Check direct server use and Remote use when shared behavior changes. Check the
actual client route and both streaming and completed JSON where supported.
Include the reverse action: connect/disconnect, start/stop, approve/revoke.
Do not claim an unimplemented workflow works because a lower-level test passes.

## Protect the running system

- Framerize is a small project for trusted friends. Keep it open at the core:
  plain account JSON, environment variables and optional tokens. Do not add
  auth layers, secret stores or blanket log redaction without a user need.
- Never commit real account sessions, tokens or captured provider payloads.
  Examples and fixtures use synthetic values.
- Use private temporary state for ordinary tests. Do not point a test server
  at live accounts or copy real sessions into fixtures.
- Stop only processes this task started, using their captured PID or handle.
  Never kill processes by a name or path pattern.
- Live inference, account import, exposure and deployment need explicit scope.

## Verify the claim

Use the smallest meaningful check for the change. Read the current package
scripts and build prerequisites; do not run repository-wide tests just to
produce a green count. Backend changes need focused behavior tests. Use
[test-framerize](.agents/skills/test-framerize/SKILL.md) for installed flows.
Keep source, installed-package, native and live results separate. An unavailable
runner is not a test failure or a pass. Retain useful unchanged evidence.
Before running a probe against a retained package or dist, name its exact path
and source revision. Compare the affected code and existing regressions with
current `main` and the active change that owns those files. If that source
already has the fix, do not rerun the stale probe or start a duplicate
implementation. Send only the relevant evidence to the fix's owner.
The `Check` job in `.github/workflows/ci.yml` is the single CI result to
require. The `main` ruleset requires this result from GitHub Actions.
Reviewers must confirm it passed on the reviewed head before merging.
Independent source review remains a maintainer responsibility because the
ruleset does not require an approval. Follow [the maintenance workflow](docs/pr-maintenance.md).
`.github/scripts/ci-changes.cjs` selects its gates; an unknown path runs them all.

## Code Review Rules

1. Preserve the client contract through both the server and Remote. Flag a
   changed route, model option, non-streaming response or tool result that
   breaks an existing caller. Trace the caller and provider mapping, not just
   the new function. Do not count a URL alias as complete provider compatibility.
2. Preserve credential and resource boundaries. Local client credentials must
   not reach the remote provider. Flag a committed real credential, incomplete
   hop-by-hop header filtering, unbounded exchanges, or cancellation that leaves
   upstream work, sockets, timers or child processes running. Test the failure path.
3. Preserve truthful delivery and readiness. Remote must not acquire server
   accounts or Castle assets. Readiness must not report success for an unusable
   configuration. Package claims must match the exact source and artifacts
   tested; an older executable does not qualify changed runtime code.

## Pull requests and documentation

- Read the actual diff and affected callers. Follow
  [review-framerize](.agents/skills/review-framerize/SKILL.md); record the exact
  reviewed commit, concrete findings and limits. Bot silence is not approval.
- Keep one coherent problem per change. Explain a simpler alternative when it
  removes a concrete cost; do not judge complexity by file or line counts.
- Create, merge or close PRs only when asked. Follow [CONTRIBUTING.md](CONTRIBUTING.md).
- Keep user instructions current. Document durable decisions, not code tours,
  agent transcripts or a second progress ledger. Keep scratch work outside Git.
- Report meaningful results together. Do not send repeated unchanged status.

These instructions adapt T3 Code's approach. See [.agents/ATTRIBUTION.md](.agents/ATTRIBUTION.md).
