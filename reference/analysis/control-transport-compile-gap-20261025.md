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
