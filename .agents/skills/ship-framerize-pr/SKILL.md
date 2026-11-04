---
name: ship-framerize-pr
description: Open, self-review, fix, and then merge or close one Framerize pull request before starting the next. Use whenever you are about to open a PR in this repository.
---

# Ship a Framerize PR

Nobody else will review your PR, and no bot will. You own it until it is
merged or closed. Read [AGENTS.md](../../../AGENTS.md) "Review your own PR".

## Before opening

1. Check that you have no other open PR: `gh pr list --author @me --state open`.
   If one exists, finish it first.
2. Keep one problem per PR. Run the focused checks that prove the change.

## Review loop

1. Open the PR with the template. Record the head SHA.
2. Start an independent review subagent that did not write the change. Give it
   the PR URL, the head SHA and [review-framerize](../review-framerize/SKILL.md).
   Ask for findings with file and line, and one verdict: merge, fix first, or close.
3. Verify each finding against the code yourself. Fix confirmed findings on the
   same branch and push. Dismiss false findings in a PR comment with the reason.
4. If a fix changed behavior, review the new head again. Repeat until the
   reviewer reports no confirmed findings.
5. Wait for required checks. Fix real failures. Do not skip or weaken a check.

## Finish

- Merge when review is clean and checks pass. Use a squash merge with a
  conventional title.
- Close when review shows the change is wrong, superseded or not worth fixing.
  Say why in a comment and link whatever replaces it.
- Before merging or closing, post one comment naming the reviewer model, the
  reviewed SHA, the findings fixed and the checks run.

Only then open the next PR.
