# Browser service restoration

Activated version: `0.5.1-alpha.13`.
Branch: `fix/verified-browser-restore-20261102`.
The package was built from `6040e344a5abd38d8fd104be805bd05220d5493f`
on the iMac Docker builder in 221.237 seconds.
The earlier `3c99f1ee` candidate passed 32 inference checks and clean shutdown but did not qualify:
the longer Claude probe exposed default reasoning truncated at 600 tokens.
The correction passed exact-package qualification. The guarded activation
observed an idle predecessor and started production at 06:02:01 UTC.

The recovered verification job stopped before inference when browser page
navigation exceeded 60 seconds. That failed prewarm is retained. The isolated
container held 1,827,196,928 bytes of setup-file cache. Advisory release of
clean private dependency and archive pages reduced total container usage
from 2,707,701,760 to 1,057,550,336 bytes without changing file contents,
account history or limits. A single preparation retry on the same account
completed successfully in 125.035 seconds.
An independent read of the login page returned HTTP 200 in 1.063 seconds.

Binary SHA-256:
`a46d96c6a43664360a08dac14bacf74a1f88116d4d19c1cad03ed03d7009377c`.
Archive SHA-256:
`d9e2e81620213579a946d9b0e256092ebc0599af46599aaf6e1cfa0e920ac1b2`.

The initial live process was PID 2767593, started at 04:10:23 UTC.
Its source was `d578e93ecc096ff3470516891a5a95c12b9762a9`.
Health and readiness returned 200. Its measured cgroup memory was
7,705,743,360 bytes under the unchanged 10,737,418,240-byte limit.
That liveness result does not prove successful inference.

At continuation, production was healthy as PID 2821815, started at
04:47:21 UTC, with the same source and 7,920,857,088 bytes of cgroup memory.
No production operation was performed during the shutdown investigation.

At 05:18:47 UTC, a separate operation restored the earlier alpha.12 package,
source `38f3c583473dea611f285ea455edceffd4331934`, as PID 2864123.
Read-only verification confirmed its complete release inventory, health,
readiness and unchanged twelve-account identity digest. The activation
procedure now pins that observed predecessor and refuses a different runtime.
All thirteen inert activation and rollback checks passed after this update.

The earlier alpha.12 isolation returned a real custom OpenAI 200 in
127.602 seconds. It then disconnected its browser without receiving the
close acknowledgement within ten seconds. Python cancelled the protocol
reader while a Playwright RPC waited for an abort acknowledgement. Captured
task stacks show that circular wait; the isolated process did not exit.

The repair stops the driver before Python cancels its protocol reader,
including after failed browser closure. Drained browsers close concurrently
within a shared 60-second acknowledgement window. An unconfirmed close keeps
its dirty history and reports whether disconnection or acknowledgement is
missing. After the driver stops, the host cancels and joins any remaining
close receipt before exiting. A forced real-driver timeout exposed this
additional ownership race in the first candidate. Its new regression fails
on that candidate. All 115 focused
shutdown, lifecycle, account-isolation, retention and rate checks pass.
Three credential-free real-browser closes also completed before the repair;
those do not qualify the repaired package or its real-request shutdown.

The exclusive saved-session account is `verification-20261102`.
Its identity and workspace are outside all twelve production accounts and
all 28 workspace assignments. Its original workspace balance was 500 credits.
Testing retains this account and workspace, with paid routes disabled.

The first long numeric probe returned an unexplained provider refusal in
2.405 seconds, with zero generated tokens. It remains a failed check.
A website-design probe then returned 600 thinking tokens and no text,
although the client allowed 2,400 output tokens. The provider generated
reasoning even without an explicit `thinking` field. The gateway recognized
only explicitly requested adaptive thinking and imposed its 600-token stage.
The correction preserves the full requested allowance on the observed Fable
5.1 route when its default thinking applies. Explicit settings still take
precedence; no effort setting is inserted or reduced. The real text-only
Claude probe subsequently returned HTTP 400: reasoning is mandatory on this
endpoint and cannot be disabled. That refusal is retained. Positive staged
continuation is verified separately on the supported OpenAI text route.

The regression reproduced the old 600-token allowance instead of 2,400.
All 127 affected Messages tests pass after the correction. Six new tests
cover provider-default reasoning, explicit text continuation, small and
uncapped allowances, preserved truncation, and failures without replay.
Timeout diagnostics distinguish default reasoning from explicit adaptive
thinking and preserve the observed upstream cause.

| Error family | Cause and repair |
|---|---|
| Long service-wide refusal periods | The divergent build retried security refusals after eight seconds, increasing protective pauses. Restore the browser lineage that preserves refusals without replay. Framer's private risk reason remains undisclosed. |
| Cached-part token generation | The divergent Camoufox path reintroduced token forging. The restored path uses the account browser's actual SDK result without rewriting tokens or inventing activity. |
| Node timeouts | Overlapping response polls and repeated pipe-buffer copies delayed the driver. Keep one owned poll through its deadline and use linear buffer copying. |
| Browser memory growth | Unused device contexts, debugger captures, finished requests, console handles and closed-page entries remained retained. Release those owners and trim unused contexts while preserving active requests. |
| Low-RAM admission 503 | Browser retention exhausted host or cgroup headroom. Restore the retention repair and retain the admission guard with actual usage and allocation figures. |
| Emission errors | Closed channels obscured producer panics, serialization errors and disconnects. Preserve the underlying emitter or producer failure. |
| Generic admission and cooldown messages | Cleanup causes and distinct device limits were discarded. Retain recorded causes and distinguish unavailable history, account state, missing scope, occupied capacity and maintenance. |
| Admission closed after a readiness timeout | Dropping a completed readiness response falsely marked inference cleanup unconfirmed. Complete the diagnostic locally while retaining ownership of unfinished checks. |
| Credit refusal reported as 500 | Preserve HTTP 402 and its credit cause. Emit typed terminal stream errors and disable paid fallback. |
| Personal-space credit-check 403 | Personal user IDs were sent to team-credit endpoints. Scan and rotate only organization spaces. |
| False startup cleanup fence | Configuration failed before any browser launched, but the journal recorded a running browser. Finish configuration first and retain real launch uncertainty. |
| Lost storage failure | Later cleanup replaced the original write failure. Retain the operating-system or SQLite cause without exposing private data. |
| Provider 429 | Rate limits were confused with workspace saturation. Preserve the provider's result and bounded retry rules on the same identity and route. |
| Valid slow output discarded | A throughput heuristic rejected successful output. Retain it within the existing active-time limit. |
| Model-list 404 | Missing dialect routes now expose the existing model registry. |
| Empty or truncated output | Validate missing host bodies and terminal statuses instead of accepting empty success. |
| Remote 400, 502 and 503 | Preserve non-JSON HTTP status and any observed provider, connection or Cloudflare cause. Do not invent an undisclosed cause. |
| Shutdown EPIPE | Signal the main process first so it can drain its children, using mixed systemd shutdown. |
| Shutdown hang | Cancelling the protocol reader before an outstanding close RPC left an acknowledgement wait with no reader. Stop the driver first, then join retained close calls. |
| Relay warnings and HTTP 401 | Treat ordinary cache misses normally. Disable the failing optional relay and verify local minting. |
| Lost log writes and malformed records | Preserve bounded I/O diagnostics and the existing safe append behavior. |
| Local timing failures | Use the full 180-second buffered window, check budgets before retries and give completed responses a separate acknowledgement deadline. |
| Reasoning-only truncation at 600 tokens | The gateway missed Fable 5.1's default reasoning. Preserve the full client allowance on that observed route and keep explicit thinking settings unchanged. |
| HTTP 400 when disabling Claude reasoning | The provider requires reasoning on this endpoint. Preserve its explicit cause and the client's setting; do not silently disable reasoning or replay the rejected request. |
| Claude upstream 504 near 60 seconds | Preserve the observed gateway timeout and prevent replay of uncertain buffered work. Local deadlines cannot extend Framer's gateway limit. |

Staged continuation requires a returned checkpoint. A buffered first request
that times out before returning one cannot safely resume its hidden reasoning.
The requested effort remains unchanged. Paid fallback remains disabled.

The underlying evidence is retained in the [recurring-error record](2026-11-01-recurring-errors.md),
the [Claude investigation](2026-11-01-claude-stalls.md), and the
[deadline verification](2026-11-02-local-timing.md). Old records with no saved
cause remain explicitly unknown; a security explanation is never invented.

The core suite passed 156 tests and the server account checks passed 65.
The browser and history checks passed 68 tests. The final storage-cause change
has its own passing regression and final rate-suite run. The final build
verifies hashes of the three Messages files covered by the 127 passing tests.
Its remaining changes from the preceding candidate are documentation.
The unchanged browser, shutdown and other Rust checks retain their earlier
evidence. Thirteen inert activation checks passed again with the new source pin.
GitHub's build and CodeRabbit checks were skipped, not passed.

| Error families | Verification evidence |
|---|---|
| Node timeouts | `test_response_poll_ownership.py` reproduces overlapping polls on the baseline; the corrected reader retains one poll. Response-queue checks also pass. |
| Cached tokens and security-refusal replay | Fresh-token, account-isolation and retry-policy regressions require the actual SDK result and preserve refusals on the same account. |
| Browser retention and RAM admission | Device-retention and busy-device fixtures, `memory_admission_tests.rs`, and the matched 512-request measurement below. |
| Emission, empty output and terminal failures | `response_output_tests.rs`, `responses_sink_tests.rs`, and Messages JSON/terminal regressions cover producer, serialization and incomplete-response failures. |
| Admission, cooldown and readiness fences | Explicit-admission, refusal-publication, readiness, rate-history and error-cause regressions preserve the actual cause and cleanup ownership. |
| Credit refusal and workspace discovery | Credit-error, credit-scan and workspace-rotation regressions; six exact-binary checks reject paid routing before any account is used. |
| Startup, shutdown and storage | Browser-startup, shutdown and rate-history regressions; a real forced-close failure stops the driver and its descendants without clearing failed-close history. |
| Provider limits, remote HTTP failures and relay failures | Messages retry-policy, error-cause and authentication-relay regressions retain the provider status and restrict replay. |
| Slow output, local deadlines and default reasoning | The 127 passing Messages tests include timing, adaptive reasoning and six new default-reasoning cases. The previous implementation fails the new allowance regression. |
| Model-list routes and log handling | Model-discovery tests and real HTTP model-list checks; retained safe-append and bounded log-write failure checks. |

The retained matched memory fixture completed 512 requests in both versions
under the same 2 GiB limit. Baseline PSS ended at 1,437,474,816 bytes; the
repair ended at 640,508,928 bytes, 55.4% lower. The repaired warm range was
638,704,640 to 665,662,464 bytes. These are one-browser local HTTP fixtures,
not a six-browser production soak. Exact-package AI-load results follow below.

The final isolated run uses a separate database and recovered profile on
port 8792, inside a 3 GiB container with two CPUs. Setup-file cache initially
consumed admission headroom: cgroup memory was 2,878,324,736 bytes while
file cache alone was 1,721,196,544 bytes. Advisory cache release on private
setup and dependency files reduced cgroup memory to 938,283,008 bytes.
No file contents, account history or resource limits changed. The same
account then completed normal browser preparation in 64.819 seconds.

The acceptance criteria were fixed before the request run: 32 sequential
successful buffered and streaming requests across both APIs, peak process
PSS no higher than 2 GiB, and the final eight-sample mean no more than
128 MiB above the mean for requests 9 through 16. Separate checks require
successful staged continuation, unchanged workspace credits and confirmed
normal shutdown before a qualification receipt can be created.

The final package passed all 32 buffered and streaming requests across both
APIs. The first cold OpenAI request took 75.460 seconds. The long Claude
request completed in 48.679 seconds with 3,037 output tokens and 671 words.
OpenAI staged continuation completed 1,084 words across two successful
upstream requests in 16.447 seconds. The unsupported disabled-reasoning
Claude request remains a recorded HTTP 400, not a passing continuation test.

Across 33 process-memory samples, peak PSS was 1,566,776,320 bytes. The mean
for requests 9 through 16 was 1,147,205,120 bytes; the final eight-request
mean was 1,146,079,744 bytes. The same workspace retained all 500 credits.
Normal shutdown completed in 15.722 seconds with exit code zero, clean
account history and no surviving descendant processes. These observations
qualify the isolated package, not a long-term six-browser production soak.

Fifteen inert activation and rollback tests pass, including a bounded idle
wait that never operates the service while requests remain active. The final
idle check follows package validation and rechecks the service generation.
Earlier production preflights encountered continuous request traffic and
refused to stop the service. The final guard found an idle interval before
operating the service. All fifteen tests passed on the final guard revision.

Production changed from alpha.12, PID 2864123, to alpha.13, PID 2923147,
at 06:02:01 UTC. Health and readiness returned HTTP 200. Both live custom
OpenAI Responses and Claude Messages returned HTTP 200 with the expected
text and successful terminal status. No rollback was needed.

All twelve account identities and the 10 GiB memory cap, 9 GiB soft limit
and 1 GiB swap cap are unchanged. Metered routes and the shared token socket
remain disabled. Both installed release inventories passed verification.
Immediate cgroup samples were 6,662,553,600 bytes before activation and
6,597,324,800 bytes after. These different live workloads do not establish
a memory reduction; the matched fixture and isolated AI measurements above
provide the memory evidence.

The journal records 29.345 seconds from the controlled stop request to the
new service starting. It reports no forced termination, and the old main
process is absent. The first 228 structured records after activation contain
no warnings, errors or HTTP failure statuses. This is an initial observation,
not a long-term production soak.

Public verification connected to a global Funnel address with TLS hostname
verification and no authentication. Both model-list routes returned 200, and
`/openai/v1/responses` returned a completed response with the expected text
in 1.175 seconds. The shared public gateway is
`https://iris-1.javanese-mercat.ts.net:10000`.
OpenAI clients use `/openai/v1`; Anthropic clients use `/anthropic`.

The final guard hashes, activation receipt, public request result and journal
transition are retained in the private outage evidence directory. The source
and documentation remain on PR #29. GitHub's skipped review is not represented
as a passing check; the local regression and exact-package evidence above
support this activation.
