# Explicit V3 endpoint alongside normal browser routes

On 2026-10-25 at 05:19 UTC, an isolated empty-store startup passed nine
checks using qualified source `52019edc` and application binary
`6a2f0a9355f4e73f2a55846eee8bee831f4044089a05304e5405d8432ad64f11`.
The packaged host was `f519bb3f`; the V3 configuration was `b581349b`.

The command supplies `--offline-worker-config` and `--offline-sdk v3`.
It supplies neither `--offline-routing-config` nor
`--browser-request-ownership`. The explicit offline configuration installs
operation ownership itself. Without the routing configuration, normal
requests continue to use the browser path.

GET and HEAD passed for all three hello routes. The account list and pool
were empty. The explicit `/api/offline/complete` endpoint rejected an empty
object with its expected missing-field response. No browser or worker child
was present at the two process checks. Source control flow rejects that input
before native work. This is not exhaustive transient-process tracing.

The accepted wrapper retained 128 MiB, no swap, 32 tasks, one CPU, private
PID/network namespaces, masked homes and the shared execution lock. All 1,941
input hashes matched before and after execution. The daemon was terminated
and collected; the supervisor reported no remaining children or limit events.
The transient unit and known processes were absent and the lock was released.

Collection SHA256:
`ea3195ca94830943b137428651da08034a266adf90c9d91f5955a4a19cc964b4`.
The exact probe is `scripts/tests/shared_service_startup/probe.py`; it requires
the bound isolated wrapper, package/config paths and a fresh output directory.
It must not be used as a production account or service launcher.

The running R9 repair was unchanged. This result proves startup compatibility,
not account admission, upstream delivery, total service memory or native
acceptance. The next runtime step needs current account, history and egress
binding within one owner. Pool exclusion alone does not transfer ownership
to a second process, so the unexecuted account-copy draft is not in use.
