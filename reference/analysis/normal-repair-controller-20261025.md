# Normal repair controller qualification, 25 October 2026

The existing repair candidate from source `93dad965` remains unchanged.
Its executable SHA256 is
`f70458d364ec624006b5dc968fda49c06ab22aa65673c56fc43b87d7d223aba3`.
All twenty accepted application cases are retained without rerunning them.

The rollout controller had a shutdown-order gap: it killed the paused sidecar
but left the server paused until systemd stop. Systemd can resume a process
during stop, allowing server code to run after the captured state was final.
The corrected sequence retires the paused sidecar and paused ingress owner
before requesting systemd stop. A readable pidfd identifies an exited child
even before it is reaped, preventing cleanup from resuming its paired server.
Confirmed termination does not substitute for final absence or an empty scope.

Fifteen focused checks passed: seven new shutdown cases and eight affected
hold/cleanup controls. They use synthetic state and inert processes in private
PID/network namespaces, with 128 MiB memory, no swap, 32 tasks and one CPU.
The test unit and cgroup were absent afterward; known launcher/client PIDs were
absent and the shared lock was free. Transient descendants were not exhaustively
captured. No service, private memory, SDK or real account operation was tested.
Result SHA256:
`480ffe97b34bfa0754a05f6ea2473eaaabba869e111f446c6991be0e83669718`.

The separate fifteen readiness/rollback cases are reused after source equality
checks. Their result SHA256 is
`b26e56c5c83785be562bf8dc1890bc293b7781588e1355e14cf175bc41f1dee2`.
The final plan binds 63 source/evidence inputs and the unchanged 15-file package.
Plan SHA256:
`a04be02ef10a45776305721c9b3479b9ed812fac289ae3a898206b39dbf3dda4`.

The user subsequently authorized this candidate, its controlled switch and two
stream checks. Fresh attempts at 02:18 and 02:21 UTC ended before any switch.
The original executable and generation remained unchanged at 02:25:32 UTC.
Both controllers and authentication helpers are collected, their units and
cgroups are absent, and the shared execution lock is free.

After the first refusal, a read-only sample found an uncertain resident with no
account users. One existing guarded idle-close request succeeded and removed
that uncertainty. The next controller still could not establish quiescence:
the host reported four active requests, and three endpoint connections belonged
to the investigation's Codex process. None was cancelled. The failure events
preserved only exception type, so their precise original messages are unknown.

These attempts are consumed. Another switch requires actual request drain and
a fresh invocation binding, without repeating accepted tests or adding a blanket
cooldown. Real state restoration, same-generation interruption/cleanup and the
subsequent complete stream remain unverified. Signup and V3 are independent.

Three additional diagnostic/privacy cases subsequently passed under the same
128 MiB/no-swap/32-task isolation. The failure recorder retains only allowlisted
codes and bounded source file/line locations; arbitrary exception arguments and
local variables are not serialized. Result SHA256:
`237308bfd3dc1e98955c8d33e1078c33595f9da25f7213f683201ee51d7c509a`.
The job is collected; it provides no new historical failure attribution.

A fresh, unexecuted controller binding includes that helper and changes only
the driver's import and two diagnostic calls. Its 64 source/evidence pins and
unchanged package were verified. The prior 30 controller and 20 application
passes are retained. At 02:30:40 UTC, the independent recovery guard was ready,
but five active requests and four established connections prevented rollout.
No request was cancelled, no authentication helper was started and no service
switch occurred. The next action is this fresh invocation after actual drain.


On 25 October at 03:08:43 UTC, the fourth attempt was fully collected.
The controller returned `positive_quiet_window_not_reached`; the authenticated
descriptor helper timed out without receiving a controller request. Both exited
1. No capture, candidate switch or acceptance POST ran. All known helper
processes and the controller unit/cgroup were absent; the shared lock was free,
source pins unchanged and the original service generation verified.
Collection SHA256:
`8f718455a5931b090abed6959e5c7095af075a93f82c41a4b5f3ce002a6922c4`.

The user had explicitly allowed stream interruption, but the consumed controller
still required `requests_may_be_cancelled=False`. The next implementation must
honor that permission while retaining genuine rate history and joined cleanup.
Simply removing the quiet predicate would violate the extractor's active-work
checks. The source-work hold was released after collection; automatic goal
pausing had not succeeded. No acceptance or upstream-refusal claim follows.
