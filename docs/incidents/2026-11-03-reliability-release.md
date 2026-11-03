# Framerize reliability release

Current release: `0.5.1-alpha.25`, source
`36db08e2fb5e9c93966dad4d33741788c2c56144`.
Live as PID 862608 since November 3 at 06:32:06 UTC. Normal shutdown
replaced alpha.21 PID 654353. Health, readiness and live Kimi, Claude and
Astra requests returned HTTP 200 with expected content and terminal status.
All twelve accounts remain eligible, with the six-browser capacity and
the original service memory limits intact. No rollback was needed.

The exact package passed 32 real buffered and streaming requests on port
8792. Signed checkpoint follow-ups passed, including after daemon restart.
A truncated tool call recovered into one complete 300-number call, with two
signed thinking blocks preserved through its successful client follow-up.
The captured 60.313-second HTTP 504 request completed in 155.338 seconds
through eight checkpoints, none longer than 21.234 seconds. Its twenty turns,
98 tools, xhigh effort and 64,000-token total allowance stayed unchanged.

The repeated-request run peaked at 1,356,983,296 bytes of PSS. Its early warm
mean was 1,318,408,576 bytes and its late mean was 1,304,232,448 bytes.
The cgroup peaked at 1,768,431,616 bytes with no limit or OOM events.
Normal shutdown took 3.012 seconds with no owned processes left. The exclusive
workspace retained all 500 credits. Six paid-route rejection checks passed.

Alpha.25 binary SHA-256:
`5a0562ca296c93d8ca529c1f8aca6cc314e01440214cffa801c942c2b09b27b9`.
Archive SHA-256:
`91b73618ade7849b2bb3aae5060e1dc1ba437afea5b5dd3eb9618f506c88912f`.
Private qualification and activation receipts are in
`native-checkpoint25-r3/evidence/` under the dated incident cache.
The iMac built this source in 400.161 seconds after a checksummed dependency
was transferred to its offline Cargo cache. All nine package checks passed.
GitHub build and CodeRabbit checks were skipped, not passing checks.

Immediate service memory samples were 5,196,029,952 bytes before activation
and 2,829,983,744 bytes after verification. These have different browser
lifetimes and workloads and do not measure memory savings. The controlled
comparison below remains the evidence for the retention repair.

Private Claude body capture remains bounded to 128 files and 64 MiB and
expires at 08:32:45 UTC. The live checkpoint directory is mode 0700 and its
database is mode 0600. Both immutable release inventories passed verification.
See [the checkpoint investigation](2026-11-03-claude-checkpoints.md) for the
verifier corrections and exact limits of the continuation change.

Previous release: `0.5.1-alpha.21`, source
`674fd36cb7a41a9eca09a1cbc13f61331905a290`.
Live as PID 654353 since November 3 at 04:31:17 UTC. Activation replaced
alpha.18 PID 507283 through normal shutdown. Health, readiness and real
Kimi, Claude and Astra responses passed. All twelve account identities,
the six-browser capacity and the existing memory limits were preserved.

Exact-package qualification on port 8792 passed 32 ordinary requests,
four requests under memory pressure, two adaptive Claude requests and a
two-request signed-thinking tool round trip. The exclusive workspace kept
500 credits. Shutdown took 3.777 seconds and left no owned processes.
Peak PSS was 1,818,568,704 bytes; the early warm mean was 1,581,199,360 and
the late mean was 1,410,245,248. This is a different workload from the
controlled memory comparison below and does not establish total savings.

The separate continuation diagnostic ended in a provider refusal with
category `reasoning_extraction`. Its second HTTP 200 contained no answer.
The diagnostic completed, but adaptive chaining did not succeed. The
gateway dropped `stop_details`, which concealed the provider's explanation.
That metadata loss is the next correction. No refused continuation is replayed.

Alpha.21 binary SHA-256:
`2b7c8fa41ede032b8e4d29edf545ad01c1073e6be6fc876eecbd5c1b4c1a0be3`.
Archive SHA-256:
`429555979cf53884c213f47c3783e83d4846f3a90c1b67144dc613a462373f78`.
Private evidence is in `native-alpha21/evidence/qualification.json` and
`native-alpha21/evidence/activation-654353.json` under the incident cache.

Previous release: `0.5.1-alpha.18`.
Source: `d857d6ee8d77dcaff349b7b14ec3b2e1ce065a15`.
Live as PID 507283 since November 3 at 02:56:53 UTC.
Health, readiness, buffered Kimi, Claude and Astra, and both streaming
dialects returned HTTP 200 with complete expected responses.

The table includes repairs retained from the previously qualified browser
releases. Those unchanged paths reuse their recorded regression evidence.
Alpha.18 adds the cold-preparation fix to alpha.17's diagnostics and warmup fixes.

| Error | Cause and fix | Verification |
|---|---|---|
| Claude adaptive requests exceeded the upstream window | Adaptive reasoning bypassed chunk sizing. Alpha.25 uses completed signed Opus 5 checkpoints, starting at 600 tokens and capped at 1,200. | Captured HTTP 504 request completed through eight HTTP 200 checkpoints, longest 21.234 seconds. |
| Signed follow-up returned HTTP 400 | Combining chunks lost their original assistant-turn boundaries. Alpha.25 restores them from persistent hashes and offsets. | Buffered and streamed combined follow-ups, a restart follow-up, and a signed tool round trip pass. |
| Optional browser extension exhausted startup memory | The default ad blocker allocated about 1 GiB of anonymous memory during startup. Exclude that optional extension through the supported browser option. | Matched startup peaks fell from 1.56 to 1.08 GB; alpha.25 completed real requests under a 2 GiB limit. |
| Browser startup cause disappeared during cleanup | Cleanup errors replaced initialization errors. Preserve both causes and report observed cgroup event increases for task-limit denials or OOM kills. | Original-source reproduction and 53 focused runtime tests; isolated task-limit failure reported its actual cause. |
| Page-navigation failures triggered inference retries | Raw navigation errors lacked a local preparation classification. Preserve navigation and warmup causes and prevent smaller-token retries. | Old-source failure reproduced; 60 runtime tests and both API-chain no-retry regressions pass. Included in alpha.25. |
| Claude refusal explanation vanished | `AssembledResponse` and stream assembly omitted `stop_details`. Carry the provider object unchanged and emit explicit OpenAI refusal fields and events. | Old-source failure reproduced; 101 protocol tests and the expanded 149 Messages tests pass. Included in alpha.25. |
| Buffered provider error lacked a cause | JSON validation discarded the provider's error object. Retain its sanitized type, code and message. | Dedicated no-replay and credential-redaction regression passes. Included in alpha.25. |
| Existing browsers deferred for low allocation RAM | Selection chose a cold account even when eligible resident browsers could serve. Select from observed residents when allocation headroom is insufficient. | Alpha.21 served four real requests under pressure without a new browser launch. |
| Browser page creation stalled | A window startup or target-registration wait held the creation queue indefinitely. Bound creation to 90 seconds and retain ownership until cleanup is confirmed. | Five driver regressions, 80 focused Python checks and alpha.21 normal shutdown pass. |
| Signed Claude history was omitted | Native Messages conversion dropped non-text thinking blocks. Preserve signed and redacted blocks in their original order. | Protocol regression and alpha.21 real signed-thinking tool round trip pass. |
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
| Unknown-completion transport failures | A failed buffered connection does not establish whether generation completed. Preserve the observed network or HTTP cause and forbid automatic replay. | Timeout and no-replay regressions. Completed Opus 5 checkpoints provide the separate continuation path described above. |
| Local timing errors | Small allowances shortened deadlines; retries and final acknowledgements used incorrect budgets. Retain the 180-second buffered window and separate completed-response acknowledgement time. | Previously qualified timing regressions. |
| Reasoning truncated at 600 tokens | Text-only staging could not retain completed reasoning. Opus 5 now preserves signed checkpoints; other adaptive models keep the caller's upstream allowance. | The expanded 149 Messages tests, real combined follow-ups and signed tool round trip pass. |
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

## Continuation limits

Alpha.25 enables qualified signed checkpoints for Opus 5. It preserves the
client's effort and total output allowance. Other adaptive models keep their
existing behavior until separately qualified. An individual upstream request
can still fail; no local setting extends Framer's deadline, and unknown
completion remains terminal. Paid routes remain disabled.

## Earlier limitation in alpha.18 and alpha.21

The earlier text-continuation path targeted 45-second chunks, below the
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
