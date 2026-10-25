# Isolated authority application startup

The matching authority application `6a2f0a93` and generated host `f519bb3f`
started successfully with an empty test account store and the bound V3 worker
configuration `b581349b`. The daemon served the expected UI assets. This did
not execute the SDK, create an authenticated account or send an upstream
inference request.

The collected startup exited 0. Peak memory was 31,850,496 bytes, with no swap
or resource-limit events. Input pins were unchanged and output hashes were
verified. The captured processes, unit and cgroup were absent after collection;
this is not an exhaustive record of transient processes. The shared execution
lock was free at collection.

Receipt SHA256:
`c1d7f45975976dd619fc219e74f355ed8b66bb6c76312618aa18f8570627c27f`.
Outer result SHA256:
`ef6a5e907836370ee6169de63029ed4c6136ff05049423149620bc9c29505614`.
Recorded collection: 2026-10-25 03:03:26 UTC.

A live V3 result still requires genuine current account/device admission,
complete nonempty output and joined cleanup through `/api/offline/complete`.
The normal stream-repair deployment remains a separate operation.
