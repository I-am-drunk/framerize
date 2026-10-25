# R6 rollout and stream evidence

The 25 October R6 rollout restored eleven accounts and passed startup with
the qualified 240-second settling budget. It made one acceptance request.
The host recorded a successful upstream completion, but the interruption
check refused `call_finished_before_disconnect`. The second request did not
run, so neither interruption recovery nor the two-request sequence passed.

Rollback completed. The collected 04:27 UTC result records the previous
binary running with automatic restart, both processes resumed, temporary
ingress restrictions removed, helpers absent and the execution lock free.
This is an actual failed rollout with successful recovery, not acceptance
of the repaired candidate or browserless behavior.

The failure report omitted the first stream's counters. The attached
`admission-stream-evidence-r1.patch` preserves snapshots at the decision
points and before cleanup. It adds separate delta and terminal-text counts
while preserving the original combined count and every acceptance rule.
Cleanup remains in a nested `finally` if snapshot collection itself fails.

All eight focused isolated checks passed with zero skips: seven new inert
cases and the affected real-socket disconnect case. Execution `4f3015` and
collection `430bf6` verified unchanged inputs and joined process cleanup.
Collection SHA256: `8350ff91f2babfff2fb6d7f3cc425925d00ed1070e4fa1d955d921fede40b132`.

The correction is qualified but has not run against the live service.
Separate counts distinguish missing deltas from deltas received by the
client; they do not alone establish when the upstream generated that text.
The frozen R6 evidence cannot recover counters that were never saved.
