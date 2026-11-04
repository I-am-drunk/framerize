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
Keep one PR open at a time. The author has it reviewed by an independent
subagent, fixes confirmed findings, then merges or closes it before opening the
next PR. See "Review your own PR" in [AGENTS.md](AGENTS.md).

## Codex code review

Codex reads the `Code Review Rules` in the root `AGENTS.md`. Use the existing
review integration when available; this guide does not configure it.
An automated review supplements source review and required tests. It does not
grant merge permission. Confirm findings against the reviewed source.
