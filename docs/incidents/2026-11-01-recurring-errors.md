# Recurring error repair, 2026-11-01

Work branch: `fix/live-recurring-errors-20261101`.
The baseline is deployed commit `1b55f131e841dc4864e176b62e691be753119fe0`,
version `0.5.1-alpha.7`. This effort does not modify the Castle worktrees.

## Initial safety check

The live service returned 200 for health and readiness. Its PID was 1739550,
with start time 2026-11-01 16:03:52 UTC. All 49 release file hashes matched
the existing manifest before and after the isolated check.

An independent release build completed in 5 minutes 12 seconds. With empty,
separate database, profiles, state and logs, and no token socket, that build
returned 200 for both endpoints on port 8788. It exited cleanly on SIGTERM.
The live PID, start time and successful health responses were unchanged.

## Confirmed fixes

The response reader reissued a poll every eight seconds even when the
previous call still belonged to an unresponsive browser driver. Cancellation
of its Python waiter did not settle the retained browser RPC. The reader now
waits for one poll until the existing response deadline. A completed failed
poll can still retry the same acknowledgement.

A scaled-clock regression produced three overlapping poll attempts on the
baseline and one on the repair. The new regression and six existing response
queue tests pass. These tests use no accounts, browser or upstream requests.

## Remaining qualification

Memory retention, cause-preserving diagnostics and output delivery are under
investigation. The live service remains unchanged. The next action is an
identical-load browser memory comparison with the guarded capture repair.
