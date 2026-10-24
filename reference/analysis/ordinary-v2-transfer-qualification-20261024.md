# Ordinary V2 transfer qualification, 24 October 2026

Source `838f7dbb` is integrated as `8b5a43c` with the newer qualified router,
startup and typed-capacity changes retained. All 22 incoming owner paths
match their bindings. All 37 core files match the earlier compiled input,
so core result `6459710f` is reused without another compile.

All 79 selected synthetic cases pass: 31 Rust (`292133be`) and 48 Python/Node
(`4f1b5d34`). The Rust selection includes repeated and concurrent preparations,
failed or abandoned transfer, original-grant/session binding, joined mint
completion and account-isolation controls. Python covers the producer,
configured launch-route consumption, six synthetic JavaScript cases, package
closure and affected admission/ownership controls. No real SDK is exercised.

Rust test compilation passes (`1884a4c5`). Host, API, store and server rebuild
(`7bbc0120`, `771249c6`, `5da6d55f`, `e1d77fe0`). Production linking passes
(`cbf5e604`); executable SHA256 is
`6012de1ce004a10be7187370c06841450734bd0a1ff0bad6537e896aa2007214`.
Rust tests peak at 25,411,584 bytes, Python/Node at 89,882,624 bytes and linking
at 457,199,616 bytes. Original caps, zero swap and resource-event checks pass.

Every launch used the outer shared execution lock. Seven Rust build/test
handles are terminal and collected; captured PIDs are absent and units/groups
removed. Python's terminal, recorded stage PID and unit/group cleanup are
verified. Its separate external launcher/all-descendant PID snapshot was
missed before the 2.93-second job completed; full external capture is not
claimed. The result and that limitation are retained without a rerun.

The explicit prepared native host must include all 11 pinned support files,
including the corrected observation helper. Package staging remains pending.
The executable has not been launched. This does not establish live SDK mint,
native Framer inference, measured egress identity or continuous page quiescence.
Normal-browser join observation and protected-service rollout remain separate.
