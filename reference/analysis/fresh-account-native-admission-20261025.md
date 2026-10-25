# Same-host fresh-account admission

A dedicated account can retain its actual rate history when moving from a
verified browser session to native execution. It no longer needs an imported
history snapshot solely to satisfy native-open.

The authority is the existing private SDK observation after successful login,
an eligible successful SDK call and joined browser closure. It binds the host,
account incarnation, user/session, primary device and launch route. Admission
also requires the original cooling/request dictionaries and a nondecreasing
last-mint time. No history is copied, cleared or synthesized. Native grants
retain the observation immediately, so minting still needs its private IDs
and rechecks the same authority after waiting. The private seed handoff keeps
the existing caller user/session checks. Prior restoration/enrollment paths
remain supported; their guards and rate policy are unchanged.

Validation on 25 October: all 15 selected synthetic cases passed, eight new and
seven directly relevant controls. Tests cover repeated mint pacing, retained
state, stale identity, replaced ledgers, real rate refusals, invalid cleanup,
and replacement during spacing. The first run exposed a preexisting missing
`asyncio` binding in the AST-based enrollment fixture; its two selected tests
passed after that fixture correction. No production correction was needed.

Evidence: `fresh-account-qualification-r2` in the investigation. Input manifest
`8978032f4590ed0acc553b1001be42cfdef2e9ec145667635d7d91d3b80a1b39`;
case result `81836aeadc7a06c9c4876d2170944e88d5ce886d79a3c3c9d5183301e9a5d059`.
The 128 MiB/no-swap/32-task isolated run used a measured 36,466,688-byte peak.
Captured child, unit and cgroup were absent; the shared lock was free afterward.

This does not adopt a separate Chrome login or prove native SDK acceptance.
The live test still needs a dedicated host-owned sign-in, packaged support and
complete native output with joined cleanup. R18 remains the transport control.
