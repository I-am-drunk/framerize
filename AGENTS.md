# Framerize

Framerize is a browserless Node server written in TypeScript. Framerize Remote
is a small local instance that forwards inference to that server. Castle
Library belongs on the server. This branch is work in progress, not a release.

## Keep the design simple

- Understand the user's problem before choosing a mechanism. Reuse working
  code when it fits; explain why a new layer or dependency is necessary.
- Keep provider accounts, Castle and upstream routing out of Remote and core.
  Share client-facing HTTP behavior rather than adding another listener.
- Keep protocol compatibility distinct from provider-specific behavior.
  Preserve the selected model, options and streaming mode; do not silently fall back.
- Prefer explicit TypeScript types at boundaries and inferred types internally.
  Narrow untrusted values from `unknown`; do not hide uncertainty with `any`.
- Planning and review do not authorize implementation. Preserve unrelated work.

## Cover the affected paths

Check direct server use and Remote use when shared behavior changes. Check the
actual client route and both streaming and completed JSON where supported.
Include the reverse action: connect/disconnect, start/stop, approve/revoke.
Do not claim an unimplemented workflow works because a lower-level test passes.

## Protect the running system

- Use private temporary state and synthetic credentials for ordinary tests.
  Do not point a test server at live accounts or copy sessions into fixtures.
- Stop only processes this task started, using their captured PID or handle.
  Never kill processes by a name or path pattern.
- Keep tokens out of command arguments, logs, screenshots and repository files.
  Live inference, account import, exposure and deployment need explicit scope.

## Verify the claim

Use the smallest meaningful check for the change. Read the current package
scripts and build prerequisites; do not run repository-wide tests just to
produce a green count. Backend changes need focused behavior tests. Use
[test-framerize](.agents/skills/test-framerize/SKILL.md) for installed flows.
Keep source, installed-package, native and live results separate. An unavailable
runner is not a test failure or a pass. Retain useful unchanged evidence.

## Code Review Rules

1. Preserve the client contract through both the server and Remote. Flag a
   changed route, model option, non-streaming response or tool result that
   breaks an existing caller. Trace the caller and provider mapping, not just
   the new function. Do not count a URL alias as complete provider compatibility.
2. Preserve credential and resource boundaries. Local client credentials must
   not reach the remote provider. Flag leaked secrets, incomplete hop-by-hop
   header filtering, unbounded exchanges, or cancellation that leaves upstream
   work, sockets, timers or child processes running. Test the failure path.
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
- One open PR at a time. Before opening the next PR, the current one must be
  reviewed and then merged or closed. See "Review your own PR" below.
- Follow [CONTRIBUTING.md](CONTRIBUTING.md).
- Keep user instructions current. Document durable decisions, not code tours,
  agent transcripts or a second progress ledger. Keep scratch work outside Git.
- Report meaningful results together. Do not send repeated unchanged status.

## Review your own PR

No outside reviewer or bot will review your PR. The agent that opens a PR owns
its review and must finish it before starting the next PR. Follow
[ship-framerize-pr](.agents/skills/ship-framerize-pr/SKILL.md).

1. Open the PR, then start at least one independent review subagent. Give it
   the PR URL, exact head SHA and the
   [review-framerize](.agents/skills/review-framerize/SKILL.md) skill. The
   reviewer must not have written the change.
2. Fix every confirmed finding on the same branch. Re-review the new head
   whenever a fix changes behavior.
3. When review finds nothing left and required checks pass, merge the PR.
   Close it instead if review shows it is wrong, superseded or not worth fixing.
4. Only then open the next PR. Do not stack an unreviewed PR on another one.

Record the reviewer, reviewed SHA and outcome in a PR comment before merging.

These instructions adapt T3 Code's approach. See [.agents/ATTRIBUTION.md](.agents/ATTRIBUTION.md).
