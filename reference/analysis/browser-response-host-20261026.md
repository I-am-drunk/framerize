# Host response integration

The acknowledged response queue is integrated on
`fix/browser-response-bounds-20261026`, based on `5bfe52c975b78f7e`.
The [result record](browser-response-host-20261026.json) binds the tested
source and outcomes. This milestone is locally qualified and undeployed.

The browser counts incoming bytes against an 8 MiB limit. It transfers
decoded text while fetching, with at most 131,072 UTF-16 units charged to
queued and unacknowledged delivery. The host acknowledges each frame only
after validating its sequence, offset, status, usage and size. Lost poll
replies replay the same frame. A final acknowledgement removes the job.

`Host._run_job` uses the same request deadline for start and collection.
Every start, poll and cancel goes through `Host._rpc`; timing out or
cancelling its waiter retains the actual RPC's account ownership. A missing
job fails delivery unless the host already received the final frame.
Terminal body failures discard partial output and retain existing failure
diagnostics. The caller still receives one complete response body.

The production reader and poll are byte-identical to the previously tested
experimental reader and acknowledgement poll. The independent twelve-case
Node acknowledgement contract passed before this integration. Its source
and result were reused without rerunning it or changing the frozen fixture.

At 01:13 UTC on October 26, 2026, all 23 focused Python tests passed,
including the existing eleven-scenario JavaScript fetch contract. Six new
host tests cover lost replies, missing jobs, rejected frames, terminal
failure, deadlines and cancelled final acknowledgements. The existing
activity and failure-policy cases remain in that selection.

The actual host path then passed against a local HTTPS server using browser
`152.0.4-beta.31`. One control and five injected RPC-loss cases each returned
the exact 8 MiB body with one HTTP request. The faults occurred before poll
execution, after the first chunk, after an acknowledgement returned another
chunk, after the final chunk and after final cleanup.

The same browser run verified oversize refusal with the observed HTTP 200
head, cancellation of a held response with peer closure, and a cancelled
final acknowledgement whose underlying RPC still prevented account close.
After that RPC settled, the real host acknowledged browser closure. All
jobs, request activities and browser descendants were released. There were
nine HTTP requests total and no unexpected fixture requests.

The browser run completed in 12.415 seconds under private network and PID
namespaces, a 2 GiB/no-swap cgroup, 512-task limit and the shared execution
lock. There were no memory or task-limit events. The unit was collected.

Four existing package/import tests passed separately. A temporary copy made
by `copy_runtime('framerize', ...)` included both JavaScript files and the
Python receiver unchanged, with matching runtime-manifest hashes. No build
or installation was needed. All 73 browser-fixture source pins matched
afterward; all three test units were inactive with no remaining main process.

Run the current local browser check with
`scripts/experiments/browser_response_bounds/host_probe.py` using the
[documented fixture isolation](../../scripts/experiments/browser_response_bounds/README.md).
The older comparison scripts retain their recorded source contracts and
must be run from the revisions bound in those measurements.

This fixture uses a temporary synthetic account and replaces only the
request origin with loopback. It does not run account startup, the SDK,
the native minter or a real Framer request. It does not measure production
memory savings or stream output to the application; Python still assembles
the complete bounded body. The earlier queue measurements remain evidence
for their exact prototype sources, not a performance result for this host.
