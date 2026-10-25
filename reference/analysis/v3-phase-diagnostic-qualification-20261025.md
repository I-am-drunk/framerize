# V3 failure diagnostics qualification, 25 October 2026

All 31 selected cases pass on source
`191dabe8ba9acd5063c645db2345b0c2f52523ed`: 16 JavaScript and 15 Rust tests,
with no selected skips. The Rust test build also passes. These are inert
diagnostic tests. No real SDK, account, token or service request was exercised.

The cases verify that supported structured logger tags refine an already
failed verdict. Malformed, ambiguous, conflicting or private data retains the
broad failure. Accessors and proxies are not evaluated. Success validation,
existing failure schemas, cancellation, deadline and cleanup precedence remain
unchanged. The earlier extracted-wrapper and local SDK evidence was not rerun.

The preparation's 28 source files match the source commit exactly. The three
jobs ran serially under the shared lock, with 128 MiB for tests, 512 MiB for
compilation, no swap, 32 tasks and one CPU. Peak memory was 34,803,712 bytes
for JavaScript, 191,135,744 for compilation and 16,441,344 for Rust tests.
No memory or task limit events occurred. Inputs and output hashes matched.

All terminal handles were collected. Known launcher, client and stage PIDs
were absent afterward, each unit and cgroup was absent, and the lock was free.
Transient descendants were not exhaustively captured. The jobs used a private
network namespace and masked private directories; host PID visibility remains
a documented containment limit.

Qualification result SHA256:
`f4545a65099eed3826f5099683ea22fc7720321144842e7250d498bc07fdd9fe`.
Frozen preparation handoff SHA256:
`92a780378a8efaf317c0f1645f235b81824b284a5082d5172cf3efdef6750ffa`.

Actual V3 execution remains separate. The service event preserves the precise
phase, while the standalone Rust measurement helper reports only `worker`.
Missing events are inconclusive. A timeout label records a diagnostic tag;
it does not establish timer firing, first cause or upstream acceptance.
