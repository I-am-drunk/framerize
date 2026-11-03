# Memory-aware browser selection

Candidate: `0.5.1-alpha.19`.
Source: `b42c9f5bff96495b5d4e56d2d8c7b6e319cdff1f`.

The scheduler could choose a cold account when an eligible loaded browser
was available. Under host memory pressure, this requested another 1 GiB
allocation and returned HTTP 503. Reserved pool slots could also refer to
absent browsers, so pool membership did not prove browser availability.

The repair observes actual browser state when startup headroom is low.
Selection and final admission validate browser identity and residency.
Request admission and selection share the same memory reservation count.
Cold accounts retain their turn for when allocation headroom recovers.
Authentication, cooldown, exclusion and ownership rules remain enforced.

A second race could report a full pool after an observed browser was parked.
It now names the lost resident slot. When no account is otherwise eligible,
admission preserves the original account or ownership cause.

## Verification in progress

All 12 targeted memory-admission checks and all 492 server unit tests pass.
The full suite reused the exact compiled test executable. The crate has no
separate integration-test targets or documentation examples.
The release build and package smoke checks pass. The packaged executable
also starts on IRIS and reports the expected version and source commit.

The isolated service runs on port 8792 with one exclusive account, separate
state and logs, and no shared token socket. All six paid-route refusal
checks pass. Its workspace had 500 credits before inference verification.

The first real request stopped before dispatch for the imported account's
saved device cooldown. The subsequent attempt timed out after 420 seconds
without an upstream dispatch. Startup prewarming reported a page-navigation
timeout. The failed attempts remain recorded.
The test container was CPU-throttled at two cores; assigning four cores and
advising release of unused dependency-file cache reduced its memory from
2,931,175,424 to 2,285,236,224 bytes but did not rescue the pending request.
Node inspection found BrowserContext.newPage waiting without a timeout;
Firefox had not replied to Browser.newPage. Normal shutdown exceeded 70 seconds.
Cancelling that exact test operation through the Node debugger released the
wait. The supervisor subsequently recorded exit 0 and clean rate history.
This intervention is diagnostic evidence, not a passing shutdown check.
No isolated inference success or activation is claimed yet.

## Remaining Claude limit

The alpha.18 diagnostic at 03:03:23 UTC captured an upstream HTTP 504 after
60.2 seconds for a 64,000-token adaptive Claude request. This is separate
from the memory-admission failures. The memory fix does not extend that
upstream deadline or establish a supported reasoning checkpoint.
