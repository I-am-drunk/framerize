# Contributing

This is a work-in-progress TypeScript implementation. Read [AGENTS.md](AGENTS.md)
for package boundaries and review rules. Agree on the intended behavior before
starting a feature or a change to product defaults. A focused bug fix should
explain the failure, expected result and why the change is needed.

## One problem per change

Include the server, Remote, tests and documentation needed for the same problem.
Do not mix in unrelated cleanup. File count is not a measure of scope. Reuse
existing implementations where they fit, and explain significant new dependencies.

## Show what works

Give reproduction steps, the focused checks run and their observed results.
State the commit, runtime and platforms covered. Explain anything not checked.
For CLI changes, show the relevant command and sanitized before/after output.
For a visual interaction, include images or a recording when those demonstrate
behavior that text cannot. Keep PR-only evidence out of the repository.

## Review before merge

An independent reviewer reads the diff and affected paths, checks the evidence,
and reports findings against the exact head. Recheck behavior affected by later
pushes. A green workflow, a bot reaction or passing triage is not approval.
Source review, installed use, native builds and live inference prove different
things. Do not replace missing evidence with a broad completion claim.

Use the PR template and a plain-language conventional title. Name the model and
harness if an agent did the work. Review the contribution, not its author's style.
Only maintainers decide whether to merge or close a PR.

## Merge requirements

The `main` ruleset requires a pull request and a successful `Check` from GitHub
Actions. Confirm that result belongs to the reviewed head. The ruleset requires
zero approvals, so independent source review remains the maintainer's job.
When agents share the author's GitHub account, record that review as a
`COMMENTED` review naming the exact commit, findings, evidence and limits.
The author's own verification is not independent review.

Before merging, read review bodies, inline threads and PR comments. Resolve
blocking findings and recheck the head. Use the reviewed PR's URL with
`gh pr merge <pr-url> --merge --match-head-commit <reviewed-head>`
to target that PR and reject a changed head.
See [the maintenance workflow](docs/pr-maintenance.md) for ownership, retries
and handling PRs that should be fixed or closed.

## Automated review

CodeRabbit reads `.coderabbit.yaml`; Codex uses the root Code Review Rules.
Both supplement independent source review and required checks. Inspect the
actual review, its commit and all findings. A green, neutral or skipped bot
check can represent a review that never ran.

CodeRabbit currently requires manual requests for public repositories with
fewer than 10 stars. Use `@coderabbitai review` when the PR is ready. After a
rate-limit response, request `@coderabbitai rate limit` and wait for confirmed
capacity before requesting another review. Do not repeat a pending request.
Provider errors, skipped reviews and bot silence grant no approval.
