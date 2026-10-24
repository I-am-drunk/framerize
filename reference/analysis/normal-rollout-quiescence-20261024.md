# Quiescence reader correction

Prelaunch reached the original-service and independent-guard checks, then
failed parsing a complete malformed line in the existing event log. It did
not start the rollout controller, capture state, stop the service or issue
either planned stream request. Terminal authentication had succeeded.

The corrected bounded reader forgets its retained memory snapshot after a
malformed or non-object complete record. Only a later valid memory snapshot
restores evidence. It preserves the 2 MiB tail bound and ignores an incomplete
final line. Existing freshness, active-request, ingress, cooldown and guard
conditions are unchanged. The old reader remains preserved in execution-r1;
the current controller selects a separate corrected copy.

Six inert cases pass, including corruption before/after a snapshot, incomplete
records, non-objects, ordinary fields and the tail bound. Result `9f2f1e24`
uses the retained 128 MiB/no-swap/32-task isolated runner. The job is collected,
captured processes and unit are gone, and the shared lock is free. Transient
descendant capture is not exhaustive.

Plan `ad5f21ad` retains the approved binary, host, stop/start and rollback scope;
only reader selection and source pins change. The old plan and unused first
authentication record are preserved. Corrected preflight reports real active
work and a cooldown ending around 23:28 UTC. No controller or live stream has
run. Fresh quiescence and a sufficient remaining hold are still required.
