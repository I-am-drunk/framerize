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

The original service generation was active and unchanged at 02:01:46 UTC.
The executable plan is absent. A new protected-service exception and local
authentication remain necessary before one controlled switch and two stream
checks. Real state restoration, same-generation interruption/cleanup and the
subsequent complete stream remain unverified. Signup and V3 are independent.
