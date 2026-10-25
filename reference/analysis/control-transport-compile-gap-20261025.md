# Control transport qualification: missing fixture input

The prepared API test compilation on immutable source `a715a41a` was attempted
once on 25 October. It failed before test discovery because the isolated source
view omitted `crates/framerize/src/observed_factory_tests/tests.rs`, referenced
by `crates/framer-api/src/observed_factory_fixture.rs:128`.

All 1,640 pinned inputs remained unchanged. The compiler and supervisor were
collected, the cgroup was absent, the shared lock was free, and no resource-limit
event occurred. None of the ten selected tests ran. This is a preparation gap,
not a demonstrated production transport defect.

The preparation owner received the exact failure and a new output reservation
to bind the missing static test-include closure on the same source. The failed
attempt remains preserved. Its correction is independent of R5 recovery and
must not be treated as a prerequisite for that recovery.

## Collected second attempt

The corrected preparation added the missing module and its child fixture,
retaining the same source and ten selected transport cases. Compilation then
advanced beyond the include failure but reached the existing 805,306,368-byte
(768 MiB) cgroup memory cap. The supervisor stopped it after 24.94 seconds.
The sampled memory events recorded 19 maximum-limit events, zero OOM events
and zero OOM kills. This was not a host-wide OOM or a test assertion failure.

Terminal 69037 completed with exit 1. All 1,642 pinned inputs remained unchanged;
the known processes and cgroup were absent after collection and the shared lock
was free. No selected test ran and no successful test binary was accepted.

The compiler log is bound by SHA256
`9ae3d2d2ea476b414e3e14cd64a0c2596d71d83afff2377e87d4e105b88bc92a`;
the failed receipt by
`2bc4603bcd24104127bb28277cfda73d13346ce3dd0eca9228e069387b0dfebf`.
A fresh preparation reservation asks for a lower-memory compile arrangement
using the same real implementation and exact cases. The memory cap, deadlines,
isolation and execution ownership remain unchanged. Both failed attempts are
preserved. This qualification remains separate from service recovery.
