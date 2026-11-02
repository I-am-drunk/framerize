# Kimi stalls and account scheduling, 2026-11-02

The alpha.13 gateway had twelve eligible saved accounts and six browser slots.
Seven accounts sent requests after its 06:02:01 UTC start; six handled almost
all traffic. The other five had saved sessions but no fresh authentication
evidence in this process generation. The service stayed untouched during diagnosis.

## Causes and repair

Automatic selection ranked current request load before browser warmth, but
warmth had no fairness limit. An idle resident always beat an equally healthy
unloaded account. At capacity, that preference could keep the same six browsers
resident indefinitely. The repair ages fresh selections. After eight passes
over the eligible pool, an overdue equal-load account outranks warmth. Existing
health, availability, account ownership and confirmed-close rules still apply.
No failed request is replayed under another account.

Starting every overdue browser together could temporarily remove all warm
capacity. While one newly selected cold account holds preparation, work or
cleanup ownership, further overdue replacements defer to equal-load warm
accounts. Ordinary load, health and safety checks still apply. The full
server suite passes 484 tests with this guard; exact source hashes match
the checked-out revision.

The global refusal guard escalates clustered security refusals from 150 seconds
to a maximum of 900 seconds. Its ten-minute quiet reset ran only when a second
account refused within 90 seconds. By then the previous refusal could never be
ten minutes old. The repair checks quiet time on the first new refusal, before
recording it. It does not cancel an existing pause or remove the guard.

A Kimi request entering the 11:58:49 UTC pause completed after 906 seconds.
An Astra request entering the same pause later also waited, for 127 seconds.
This was a shared admission wait, not evidence of model-specific generation
latency. The ordinary Kimi completion median in the inspected window was 5.6 seconds.

Most HTTP 403 refusals came from two accounts that also completed hundreds of
requests. Browser token minting succeeded throughout the inspected generation.
Framer did not disclose its private reason for those security refusals.

The model catalog describes supported routing, not instantaneous admission
health. Kimi remains supported and completed requests during this generation.
The existing protocol names Framer's upstream HTTP 403 in the error message
and emits a terminal failure if streaming has already started.
Suppressing that failure would misreport an unsuccessful request as complete.
The gateway does not replay that request under another account. Client-side
retention of partially received text remains the client's responsibility.

## Verification before packaging

Both new regressions failed on the original source. The scheduler used only
six of twelve accounts after 120 sequential fixture requests. A new refusal
cluster after quiet still produced a 900-second pause. Both pass after repair.
All 159 core, 95 protocol and 483 server tests pass. The server run initially
found four older fixtures assigning credits to a personal space. They now
model incomplete initial metadata followed by the full organization listing.
The credit-admission implementation is unchanged.

Twenty-two inert activation and rollback checks pass. The activation guard
requires all recorded inference and cleanup operations to finish, including
requests waiting without a resident-browser lease. It also waits for a logged
global refusal pause to expire. A post-activation security refusal is recorded
as deferred verification; it cannot count as successful live delivery.

## Combined alpha.14 package

The iMac built clean source `2b4d01951998614d160b3d8700fcad916452d9f9`
in 191.472 seconds. All nine package checks passed. The binary SHA-256 is
`8963ac2b7fd151159e289a6750dff8c6ba52b7e7cb490b21ab48a81bf60e7972`.
The archive SHA-256 is
`d7cbfb91c0705f9539c315e0871d73818a90e50766f9f62b15e8bd44802022dc`.

Isolation exposed another source of memory pressure: browser font scanning
left unused file-cache pages charged to its cgroup. Advising release of the
1,515 font files on the preceding package reduced the same running container
from 2,686,869,504 to 1,227,100,160 bytes. This measured file-cache effect is
separate from the retained browser-object leak repaired in alpha.13.
The new package advises releasing this cache after initial SDK warmup.
It preserves font contents and reports advisory failures. All 26 focused
font-cache, startup, shutdown and memory-diagnostic tests pass.

The preceding package completed a real Kimi request in 78.377 seconds after
the diagnostic cache release. Its normal shutdown took 11.625 seconds with
no surviving descendants. These are diagnostic results, not qualification
of the new package. The same exclusive account and rate history are retained.

The new package's isolated prewarm completed in 406.378 seconds. Its automatic
font-cache advice succeeded for all 1,515 files. Its first normal Kimi request
returned a completed HTTP 200 in 67.731 seconds. All 32 sequential requests
passed, covering Kimi, Astra and Claude with buffered and streaming replies.
After the cold request, the median was 2.803 seconds and the maximum was
6.523 seconds. Every response had expected text and a successful terminal
status. All six exact-package checks rejected paid routing before account use.

Across the 33 request-boundary samples, peak process PSS was 1,206,981,632
bytes. The mean for requests 9 through 16 was 1,168,894,976 bytes; the final
eight-request mean was 1,145,025,024 bytes. The workspace retained all 500
credits. The isolated browser binary and patched archive hashes match the
browser used by the unchanged live service. Normal shutdown completed in
16.431 seconds with exit code zero, clean account history and no surviving
descendants. The exact-package qualification receipt passed validation.
Its evidence includes the automatic font-cache event, browser hashes and
215 memory samples spanning startup, requests and shutdown.

The six-browser production service remained PID 2923147 throughout
qualification, with its 06:02:01 UTC start time and intact release inventory.
All twelve accounts were enabled and available at the latest audit. Seven
had served requests; six handled almost all of them. Kimi had 3,883 successful
chains and 230 failures, while Astra had 566 successes and 20 failures.
The ten emission failures in that generation all recorded client cancellation.
GitHub build and CodeRabbit checks were skipped, not passed. Local and iMac
test evidence is recorded above. Verified activation is recorded below.

The earlier identical-load comparison remains valid for the unchanged
retention repair: 512 requests, 1,437,474,816 bytes final baseline PSS and
640,508,928 bytes after repair. This and the new real-request run measure one
isolated browser. Neither is a long-term six-browser production soak.

The complete earlier error-family, root-cause and verification record is in
[the browser restoration report](2026-11-02-restored-browser-service.md).
It includes Node polling, browser retention, admission and emission causes,
readiness ownership, credits, route timing, storage, relay and shutdown fixes.
Those unchanged code paths retain their recorded regression evidence.

## Verified production activation

Alpha.14 started at 2026-11-02 14:31:56 UTC as PID 3565455, replacing
alpha.13 PID 2923147. The guard observed no live request or cleanup ownership
and no active global security pause before stopping the predecessor.
The journal records 8.818 seconds from the controlled stop request to the
successor starting. The old process exited; no rollback was needed.

Live Kimi Responses, Claude Messages and Astra Responses all returned HTTP
200 with expected text and successful terminal status. Health and readiness
also returned 200. Both public model-list routes returned 200 through the
existing Funnel. The public proxy does not expose `/api/health`, which returns
404 there; local health and readiness remain the service checks.

All twelve production identities and six browser slots are preserved.
Both immutable release inventories pass verification. MemoryMax remains
10,737,418,240 bytes, MemoryHigh 9,663,676,416 bytes and MemorySwapMax
1,073,741,824 bytes. Automatic restarts remain zero. Metered routes and the
shared token socket remain disabled.

The activation captured 6,179,840,000 bytes of cgroup memory before and
6,852,952,064 bytes after. These different production workloads do not
measure savings. The matched fixture and isolated request measurements above
provide the memory evidence.

The first new-generation errors explicitly report client cancellations.
The scheduling regression proves all twelve equally eligible accounts receive
work with six residents. A live twelve-account turnover and long-term memory
soak are not inferred from the short post-activation window.
