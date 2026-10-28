## Problem

Describe the failing behavior and a concrete trigger. Link the bug or evidence.

## Change

Explain the resulting behavior and why these paths belong in one change.

## Verification

List the checks that ran against this commit and their outcomes. Separate
source review, fixture results, packaged-binary results and live observations.
State any skipped, failing or unrun cases. Include cleanup and existing-stream
behavior for lifecycle changes, and direct command-entry tests for scripts.

## Release and recovery

State the version and source commit. Does this change account/session/history
storage, service startup or shutdown? Describe compatibility and recovery.
Link the exact package and checksums when available. A PR does not authorize
a production restart. Do not enable automatic merging.

## Review

Record the independent review and resolved findings. An AI review is advisory;
the author must not treat it as a human approval or bypass failing CI.
