# Claude failure attribution

## Private body capture candidate

The alpha.21 candidate adds explicitly enabled private Claude request and
response capture. It records prepared payloads before dispatch and the actual
returned body or failure, with the same job identity as the network event.
Unreadable upstream bodies remain unavailable. HTTP status alone is never
recorded as a complete response.

The two-hour switch lives outside the immutable release. Files have mode
0600 in a mode-0700 directory, retain at most 64 MiB, and exclude authentication
fields. The writer has two queued records and one active write; disk failures
and a full queue cannot block inference. Browser debugger payload retention
stays disabled. All 55 focused capture, dispatch, network and response-ownership
checks pass. The candidate is not yet activated and does not implement adaptive
thinking continuation. The next step is real isolated body capture and a
signed-thinking continuation probe before qualifying the resulting release.

Branch `fix/claude-network-diagnostics-20261103` has activated alpha.18.
Source `d857d6ee` is live as PID 507283 since November 3 at 02:56:53 UTC.
Health, readiness, buffered Kimi, Claude and Astra, and both streaming
dialects pass. The user approved the incomplete isolated qualification
and explicitly waived waiting for quiet traffic before normal shutdown.
Diagnosis began with alpha.14, PID 3565455, started November 2 at 14:31:56 UTC.
The alpha.17 activation below rolled back to alpha.14, PID 468247,
at November 3, 02:34:52 UTC. A later alpha.18 attempt also rolled back
after verification selected a second account without browser memory headroom.
Final verification pins one account across routes. Details and the complete
error inventory are in [the release report](2026-11-03-reliability-release.md).

The reported November 3 failures ended at 01:05:56 and 01:07:12 UTC.
Their browser fetches failed before response headers after 65.250 and
64.117 seconds. Both had a 180-second local response deadline and zero
queue wait. Total client operation times were 75.059 and 74.416 seconds.
Those records cannot distinguish an upstream timeout from another network
failure. Elapsed time alone does not establish an upstream HTTP status.

The retained-body memory repair disabled debugger upload copies, but
`NetworkFailure` still used those copies to match network events to turns.
It consequently discarded the browser's actual HTTP status and error code.
The repair passes only the real upload's bounded turn ID through the private
browser protocol. It changes no outgoing headers, tokens or request bodies.
Uncloneable uploads are not read or repositioned by optional diagnostics.
An added regression failed on the initial candidate and passes after this
safety correction. All 17 focused Python checks pass without skips.

Matched real-browser fixtures each completed 516 local requests. The old
browser lost both hidden HTTP statuses and a connection-reset cause. The
candidate reports concurrent HTTP 504 and 502 responses independently and
reports `NS_ERROR_NET_RESET` for the disconnected request. Payload hashes
match at the receiving server, body capture stays disabled, and all request
listeners are released. Final-quarter mean PSS was 556,312,704 bytes before
and 560,585,216 bytes after. Candidate peak PSS was 650,078,208 bytes, and
its final-quarter mean was below its second-quarter mean. This is a local
single-browser measurement, not a production inference or six-browser soak.

Requested thinking, unmetered routing, and unknown-completion replay guards
are unchanged. This patch repairs attribution; an upstream generation limit
cannot be extended by increasing the gateway's local timeout. A returned
checkpoint is required before staged continuation can safely resume work.
Exact-package isolated inference qualification remains incomplete; live
activation and short inference verification are complete. Upstream timeout
prevention remains unresolved. A 60-second local timer cannot create a
checkpoint from a buffered response whose connection has already failed.

The alpha.16 qualification stalled during preparation in the emulated x86
verifier before any inference dispatch. Normal shutdown did not complete
within 70 seconds. Its verified test browser was terminated, and all eleven
owned processes exited before the exclusive account moved to native x86
verification. The account retains its history and 600-second recovery hold.

The audit also found unbounded SDK warmup calls. Two new regressions reproduced
an indefinite wait and a failed warmup returning an unprepared device.
Warmup now has a 120-second deadline. Failure or cancellation closes the
unused device context before releasing ownership. The saved device identity
stays unchanged. Failed closure keeps the account fenced. Initial account
warmup failures also propagate through the existing browser cleanup path.
The 80 existing lifecycle checks and all five warmup regressions pass. The
additional mint regression confirms that timed-out preparation also releases
the mint lock and pending-mint record. A real-browser fixture joined a never-settling
warmup and confirmed context cleanup in 1.614 seconds with a shortened test
deadline. Production warmups observed that day had a 2.331-second median
and 5.464-second maximum across 71 launches. These are preparation deadlines,
not changes to inference timeouts or requested reasoning.

Verification cleanup also missed children created by worker threads because
it read only the main thread's Linux children file. The shared verification
helper now inspects every thread. Its regression launches a real child from
a worker thread and passes. The native predecessor's corrected check observed
ten owned processes and confirmed a clean exit with none remaining in 1.756
seconds. The activation guard also includes log rotation after midnight;
all 23 activation and rollback fixtures pass. These test-tool corrections
do not change the application built from `8f320e2a`.

## Alpha.17 package and shortened qualification

The iMac completed the exact alpha.17 package in 256.159 seconds.
All nine package lifecycle checks and six paid-route refusal checks passed.
The staged binary returned HTTP 200 for isolated health and readiness on
port 8792. All 38 packaged runtime files match the reviewed source.
Binary SHA-256:
`aafdc83aa989694565656fa62f83dba6594e95f017434c4e7363f123610d2e21`.

The real-request check was refused before dispatch because host memory
headroom was 867.86 MiB and browser startup required 1,024 MiB. The isolated
workspace still had 500 credits. No isolated inference success is claimed.
The user then explicitly authorized replacement without waiting for full
qualification. That exception is recorded separately from passing evidence.
The isolated process has exited; the production account inventory is intact.

The first activation attempt found continuing request traffic and exited
without restarting production. Its follow-up reads only new operation-log
records, so it can check short idle intervals without rescanning old traffic.
Five new controls cover partial records, log rotation, missing or truncated
history, and missing operation identities. All 23 prior activation and
rollback controls still pass. At that stage the swap still required an idle
service. The later explicit user approval waived that prerequisite.

The next guarded attempt reached an idle interval but stopped before any
production change. `journalctl --grep` returned status 1 with no output or
diagnostic text because no security-pause record matched. The guard had
treated that documented empty result as a failed query. Its correction
accepts only that empty result. Permission errors, unexpected output, other
exit statuses and actual unexpired pauses still prevent activation.
The new regression fails before the correction. All 26 activation tests
and five operation-history tests pass afterward. This changes the private
activation procedure, not the staged binary or its qualification claims.

## Repairs retained from the deployed baseline

Alpha.17 retains the previously qualified Node polling, memory retention,
admission, emission, readiness and shutdown repairs. Their individual root
causes and regression evidence are recorded in the
[complete recurring-error record](2026-11-01-recurring-errors.md) and the
[browser restoration record](2026-11-02-restored-browser-service.md).
The [account scheduling record](2026-11-02-account-scheduling-and-pauses.md)
covers bounded warm-account preference, refusal-pause reset and font cache.

The identical-load memory comparison completed 512 requests in each version.
Final process PSS fell from 1,437,474,816 to 640,508,928 bytes, with retained
pages reduced from ten to three and an active stream preserved. This is
the measured effect of the earlier retention repair, not a new alpha.17 saving.
The 516-request alpha.17 diagnostic comparison above verifies that retaining
the small request identity does not restore the old payload retention.

Production observation on alpha.14 covered 1,311 memory samples over almost
eleven hours. Cgroup memory ranged from 4,263,845,888 to 8,219,025,408 bytes
under the unchanged 10 GiB limit. At most six browsers were resident.
This workload was not controlled, so it is not an identical-load comparison.
The later log audit found 74 emission failures explicitly caused by client
cancellation. No undisclosed upstream reason is inferred from those events.

## Cold preparation blocked status during activation

The corrected guard reached an idle interval and activated alpha.17 at
02:34:46 UTC. Health and readiness passed. Its first inference then failed:
preparing the new browser runtime synchronously blocked the host command
loop, so a browser-status request exceeded its five-second deadline.
The resulting unconfirmed-operation fence closed admission. The guard
restored alpha.14 at 02:34:52 UTC; no failed-release child remained.

The host now prepares dependencies and the patched browser in one tracked
worker task. The command loop can answer status while that work runs.
Concurrent accounts share the same task. Cancelling a waiter cannot start
another preparation, and shutdown joins the retained task before exiting.
Failure retains its preparation cause and publishes no partial runtime.

A controlled blocked-preparation regression reproduces the command-loop
stall on alpha.17. All 82 targeted startup, lifecycle, device, warmup and
shutdown tests pass after the repair, including five new regressions.
The exact alpha.18 package subsequently passed all nine package checks,
40 isolated health/readiness checks and the live checks recorded above.
The remaining isolated inference gap and activation approval are retained
in the release report; these checks do not qualify long adaptive reasoning.
