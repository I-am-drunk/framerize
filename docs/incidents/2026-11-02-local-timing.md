# Local deadline repair

Branch: `fix/claude-stalls-20261101`. Candidate: `0.5.1-alpha.12`.

The audit found three independent local timing defects. A small output-token
allowance shortened browser response deadlines to 60 seconds. Messages
retries could dispatch after the 300-second active budget. Finally, a fully
received response could lose its result when its acknowledgement shared the
expired generation deadline.

Production logs contain both 60-second and 180-second local poll expirations,
as well as active-budget failures. These are distinct from the measured
upstream Claude HTTP 504 at 60.122 seconds.

Buffered requests now receive the full 180-second response window regardless
of their token allowance. Explicit upstream streaming retains its 840-second
window; custom routes still prohibit that metered fallback. Starting the
in-page fetch retains its 30-second limit. A completed response receives a
separate two-second acknowledgement window, followed by the existing bounded
cleanup path. Unknown cleanup continues to hold its ownership fence.

Both chains check the 300-second active budget before every new attempt,
including retries inside a local cooldown wait. Backoffs and inter-chunk
delays check cancellation and expiration every 100 milliseconds. A response
already in flight can finish successfully after the active budget expires.

Local cooldown waits remain excluded from active time and retain their
600-second cumulative bound. Admission retains its separate 960-second wait
budget. The browser launch, host command and public proxy idle limits remain
600, 900 and 1,200 seconds respectively. Public streaming progress frames
continue to prevent an idle connection while admitted work runs.

Deadline errors now identify the response phase, including a stalled browser
poll or missing acknowledgement, and preserve any observed upstream HTTP
status. Budget errors retain the preceding failure. Logs record the browser
fetch phase separately from preparation and queueing, plus its configured
deadline. The fetch phase includes dispatch, reading and cleanup.

Two browser deadline regressions failed on the previous implementation.
All three new Messages retry regressions also failed on the previous Rust
source by observing an extra upstream attempt. The corrected browser code
passes 63 focused tests, including response ownership and listener cleanup.
The iMac qualification passed all 272 affected API tests and all 95 protocol
tests. Both recorded return codes are zero, and the logs confirm compilation
of the changed crates. The 17 transferred source hashes match this checkout.
Exact-source packaging is the next step. No production restart occurred
during diagnosis or qualification; alpha.11 retains PID 2577096.

Evidence is kept in
`/home/tjninja/.cache/framerize-claude-stalls-20261101/timing-r1`.
The earlier memory repair and its matched-load measurements are unchanged.
Requested effort, account ownership and the prohibition on paid routes are
unchanged. This patch cannot extend Framer's own upstream response timeout.
