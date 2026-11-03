# Claude failure attribution

Branch `fix/claude-network-diagnostics-20261103` prepares alpha.17.
Alpha.14 remains live as PID 3565455, started November 2 at 14:31:56 UTC.
Health and readiness pass. No production restart occurred during diagnosis.

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
Exact-package inference qualification and activation remain outstanding.

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
All 84 affected tests pass. A real-browser fixture joined a never-settling
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
