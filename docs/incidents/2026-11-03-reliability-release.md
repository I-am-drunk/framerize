# Framerize reliability release

Release: `0.5.1-alpha.18`.
Source: `d857d6ee8d77dcaff349b7b14ec3b2e1ce065a15`.
Live as PID 507283 since November 3 at 02:56:53 UTC.
Health, readiness, buffered Kimi, Claude and Astra, and both streaming
dialects returned HTTP 200 with complete expected responses.

The table includes repairs retained from the previously qualified browser
releases. Those unchanged paths reuse their recorded regression evidence.
Alpha.18 adds the cold-preparation fix to alpha.17's diagnostics and warmup fixes.

| Error | Cause and fix | Verification |
|---|---|---|
| Node timeouts | Overlapping polls and repeated pipe-buffer copying delayed the driver. Keep one owned poll and copy frames linearly. | The baseline made three overlapping polls; the repaired regression made one. |
| Stalled SDK warmup | Preparation had no deadline and could return an unprepared device. Apply a 120-second deadline and join context cleanup. | Five warmup regressions; real-browser cleanup in 1.614 seconds. |
| Admission closed during cold startup | Synchronous browser preparation blocked status replies past five seconds. Use one tracked worker task and join it on shutdown. | 82 targeted checks pass. Exact-package preparation took 5.342 seconds while 259 status checks stayed below 34 ms. |
| Browser memory growth | Unused contexts, debugger payloads, request records, console handles and closed-page entries remained retained. Release those owners without closing active streams. | Identical 512-request comparison and retained-page counts below. |
| RAM admission 503 | Browser retention and unused font cache consumed headroom. Repair retention, advise releasing unused font cache, and report measured headroom and allocation needs. | Memory regressions and prior real-request qualification. Host-wide pressure still legitimately defers new allocations. |
| Vague admission errors | Account state, cleanup failure, scope, capacity, maintenance and history failures shared generic messages. Preserve each observed cause. | Admission, ownership, history and error-cause regressions. |
| Emission failures | Closed channels hid producer panics, serialization failures and client disconnects. Preserve the underlying failure. | Output and sink regressions; 74 observed production emission failures explicitly recorded client cancellation. |
| Readiness timeout closed admission | A dropped completed diagnostic response was mistaken for unconfirmed inference cleanup. Complete diagnostics locally and retain unfinished ownership. | Both readiness regressions and the qualified server suite pass. |
| Credit refusal became HTTP 500 | HTTP 402 lost its meaning, including in streams. Preserve the credit error and send a typed terminal stream error. | Credit-error regressions and six exact-binary paid-route refusal checks. |
| Personal-space credit-check HTTP 403 | Personal user IDs went to organization credit endpoints. Discover and rotate organization workspaces only. | Credit-scan and workspace-selection regressions. |
| Provider HTTP 429 | Rate limits were mistaken for workspace saturation and could trigger payload-halving retries. Preserve the limit and bounded same-identity retry rules. | Provider-policy regressions. |
| Valid slow output discarded | A throughput heuristic rejected successful output. Retain valid output within the active-time limit. | Slow-response regressions. |
| Model-list HTTP 404 | Compatibility routes were missing. Serve the existing model registry through both API dialects. | Model-discovery tests and previous real endpoint checks. |
| Empty or truncated success | Missing bodies and terminal failures were accepted or misreported. Validate the envelope and retain the actual terminal status. | Responses and Messages body, output-limit and terminal regressions. |
| Remote HTTP failures lost their cause | Non-JSON errors lost status and provider details. Preserve the observed HTTP status and sanitized message. | Error-envelope regressions. |
| Claude `NetworkError` hid HTTP status | Disabling debugger body copies also removed request matching. Carry only the existing bounded turn ID in private browser metadata. | 17 focused tests and 516-request browser fixtures retain HTTP 502, HTTP 504 and connection-reset causes without changing payloads. |
| Long Claude upstream timeouts | Some upstream buffered requests exceed Framer's time window. Preserve observed causes and forbid replay when completion is unknown. | Timeout and no-replay regressions. This cannot extend Framer's own deadline. |
| Local timing errors | Small allowances shortened deadlines; retries and final acknowledgements used incorrect budgets. Retain the 180-second buffered window and separate completed-response acknowledgement time. | Previously qualified timing regressions. |
| Reasoning truncated at 600 tokens | Provider-default reasoning was treated as text-only staging. Preserve the requested allowance on the observed reasoning route. | The old source fails the allowance regression; 127 affected Messages tests passed. |
| Accounts excluded from rotation | Warm-browser preference could starve eligible cold accounts. Bound that preference and retain load, ownership and cooldown rules. | Scheduling regressions and observed use of all twelve production accounts. |
| Excessive global refusal pauses | The quiet-reset condition could not run at the right point. Reset the pause ladder after the required quiet period. | Baseline reproduction and core/server pause regressions. |
| Security-refusal replay and cached tokens | A divergent build reintroduced unsafe behavior. Restore the browser lineage that uses actual SDK results and preserves refusals. | Fresh-token and account-isolation regressions. No forged tokens or invented browser activity. |
| Shutdown broken pipe | Children received termination before the gateway drained. Signal the main process first with mixed systemd shutdown. | Lifecycle, activation and rollback checks. |
| Shutdown hang | A close acknowledgement waited on a protocol reader already being cancelled. Stop the driver, then join retained close calls. | Shutdown regressions and prior exact-package normal shutdown. |
| False startup cleanup fence | Configuration failed before launch but history recorded a running browser. Complete configuration before recording launch uncertainty. | Startup and history regressions. |
| Lost storage and log failures | Later cleanup hid the first write failure; log writes could fail silently. Preserve sanitized I/O causes and safe append behavior. | Storage, shutdown-cause and log-write regressions. |
| Relay warnings and HTTP 401 | Cache misses looked like failures; the optional relay credential was rejected. Treat misses normally and keep the unused relay disabled. | Relay regressions and previous verified local minting. |
| Malformed history and unsupported requests | Historical format mismatches and invalid inputs were obscured. Retain format validation and report the actual input or method error. | Existing history-preservation and API validation checks. |

Framer's undisclosed security decision cannot be inferred from an HTTP 403.
Old network failures without captured status remain unknown. Neither elapsed
time nor a blocked request is presented as proof of a hidden upstream cause.

## Memory evidence

Under the same 512-request local workload and 2 GiB limit, final browser PSS
fell from 1,437,474,816 to 640,508,928 bytes, a 55.4% reduction. Retained pages
fell from ten to three while another stream remained active.

The diagnostics change completed matched 516-request fixtures. Final-quarter
mean PSS was 556,312,704 bytes before and 560,585,216 after. Candidate peak PSS
was 650,078,208 bytes. Payload capture remained disabled and listeners were
released. These measurements cover one browser under controlled local load.

Production observations on alpha.14 covered 1,311 samples over almost eleven
hours, with at most six browsers and 4.26 to 8.22 GB of cgroup memory. That
uncontrolled traffic is not an identical-load comparison. Alpha.18 retains
the memory repairs; its short verification is not a new production soak.

## Alpha.18 verification

The iMac built the exact source in 254.291 seconds. All nine package checks,
82 targeted runtime checks and six paid-route refusal checks passed.
Cold runtime preparation took 5.342 seconds while 259 status checks stayed
responsive, with a maximum latency of 0.0335 seconds. Shutdown completed.
The isolated daemon also passed 40 health/readiness checks on port 8792.

The exclusive workspace retained all 500 credits. Its real inference check
was refused before dispatch because host admission headroom was 340.27 MiB
and a new browser required 1,024 MiB. No isolated inference success is claimed.
The test process then exited normally in 0.251 seconds with no descendants.
The user's explicit approval permits activation with this recorded gap.

The final activation procedure passed 32 regression checks and five history checks.
It preserves non-JSON error causes, distinguishes an empty journal query from
a failed query, and binds pending operations to the current process generation.

## Activation and limits

The earlier alpha.18 activation rolled back after its second verification
request selected another account and needed 1,024 MiB for a browser while
only 302.34 MiB of admission headroom remained. Alpha.14 recovered as PID
502787 at 02:55:03 UTC. This was a real admission deferral, not successful
Claude delivery or a browser-startup regression.

The final procedure pins one account across the three verification routes.
The user explicitly authorized normal shutdown without waiting for quiet
traffic. Security pauses, immutable inventory checks, identity checks and
automatic rollback remain enforced. The swap moved from PID 502787 to 507283.
All twelve account identities and the six-browser capacity are preserved.
MemoryMax remains 10 GiB, MemoryHigh 9 GiB and MemorySwapMax 1 GiB.

The first streaming check was refused before dispatch by the host RAM
watermark. Its exact request had negative 77.37 MiB admission headroom.
It remains a failed check. After headroom recovered, the same verifier
completed OpenAI SSE in 0.986 seconds and Claude SSE in 1.764 seconds,
with expected text and successful terminal events. No restart was needed.

Immediate cgroup samples were 5,076,836,352 bytes before activation and
4,834,988,032 bytes after the buffered checks. These samples have different
traffic and browser lifetimes; they do not measure a memory improvement.
A subsequent process snapshot also found 34 REA MCP helper processes using
3,366,850,560 bytes of summed RSS outside the service. Host memory pressure
is therefore not attributable solely to the Framerize cgroup. Those other
sessions' processes were not terminated or reconfigured.

## Remaining Claude continuation limitation

The current text-continuation path targets 45-second chunks, below the
observed upstream failures near 60 seconds. Adaptive and provider-default
reasoning explicitly bypass that token sizing and use the client's full
output allowance. A 64,000-token adaptive request can still time out.
Earlier controlled probes returned only thinking at a 600-token limit, with
no text checkpoint; this route also rejected disabling reasoning. Restoring
that limit would reproduce the earlier empty-answer failure.

This release does not add a supported adaptive-reasoning checkpoint or
extend Framer's timeout. Changing a local timer to 60 seconds cannot recover
a result that the upstream buffered connection never returned. Requested
effort remains unchanged, paid routes remain disabled, and unknown-completion
requests are not replayed. The next investigation must establish a supported
reasoning continuation or a verified transport with the same semantics.

Private receipts are `activation-507283.json`,
`approved-active-restart-alpha18.json`, `activation-active-tests.log`,
`live-streaming-r4.json`, `live-streaming-alpha18-r2.json` and
`alpha18-memory-admission.json` under the dated incident evidence directory.

Binary SHA-256: `68f32332e94ba360b645af855c93a4da1e17a47b9f40e1c4c9dbf365cc50fe5b`.
Archive SHA-256: `ff08b6bbdeacf1ab0776c249d52f60474cd25c9b55e32137b5fe25e3d989d8be`.
