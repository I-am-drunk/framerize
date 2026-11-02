# Browser service restoration

Candidate version: `0.5.1-alpha.13`.
Branch: `fix/verified-browser-restore-20261102`.
The package was built from `3c99f1ee85493e3cb194e0fff1eca00c0d5a5ce1`
on the iMac Docker builder in 175.675 seconds.
That package passed 32 inference checks and clean shutdown but did not qualify:
the longer Claude probe exposed default reasoning truncated at 600 tokens.
The correction requires a new build and exact-package qualification.

Binary SHA-256:
`cb374ae2b6a0a77e46dd77fd760bd75e498331d5c36fef6ead428ef15e10d70e`.
Archive SHA-256:
`b4a02752e9409570c4d54d6e599021d091a91fb30bf5ff6191407360c4cbefc1`.

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
precedence; no effort setting is inserted or reduced. Text-only continuation
is tested separately with a client that explicitly requests disabled thinking.

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
| Relay warnings and HTTP 401 | Treat ordinary cache misses normally. Disable the failing optional relay and verify local minting. |
| Lost log writes and malformed records | Preserve bounded I/O diagnostics and the existing safe append behavior. |
| Local timing failures | Use the full 180-second buffered window, check budgets before retries and give completed responses a separate acknowledgement deadline. |
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
reuses Rust evidence only after verifying that its intervening source diff
contains the two Python rate files. Thirteen inert activation checks passed.
GitHub's build and CodeRabbit checks were skipped, not passed.

The retained matched memory fixture completed 512 requests in both versions
under the same 2 GiB limit. Baseline PSS ended at 1,437,474,816 bytes; the
repair ended at 640,508,928 bytes, 55.4% lower. The repaired warm range was
638,704,640 to 665,662,464 bytes. These are one-browser local HTTP fixtures,
not a six-browser production soak. Exact-package AI-load results are pending.

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
