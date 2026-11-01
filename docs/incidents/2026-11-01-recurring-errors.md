# Recurring error repair, 2026-11-01

Work branch: `fix/live-recurring-errors-20261101`.
The baseline is deployed commit `1b55f131e841dc4864e176b62e691be753119fe0`,
version `0.5.1-alpha.7`. This effort does not modify the Castle worktrees.

## Initial safety check

The live service returned 200 for health and readiness. Its PID was 1739550,
with start time 2026-11-01 16:03:52 UTC. All 49 release file hashes matched
the existing manifest before and after the isolated check.

An independent release build completed in 5 minutes 12 seconds. With empty,
separate database, profiles, state and logs, and no token socket, that build
returned 200 for both endpoints on port 8788. It exited cleanly on SIGTERM.
The live PID, start time and successful health responses were unchanged.

## Confirmed fixes

The response reader reissued a poll every eight seconds even when the
previous call still belonged to an unresponsive browser driver. Cancellation
of its Python waiter did not settle the retained browser RPC. The reader now
waits for one poll until the existing response deadline. A completed failed
poll can still retry the same acknowledgement.

A scaled-clock regression produced three overlapping poll attempts on the
baseline and one on the repair. The new regression and six existing response
queue tests pass. These tests use no accounts, browser or upstream requests.

## Browser retention repair

The live six-browser limit held while pages grew from 17 to 37. Account-wide
idle checks prevented trimming unused device contexts during other requests.
The candidate retains each device through its actual browser calls and closes
unused contexts after completion, including while another device is busy.

The pinned debugger runtime also retained response copies, finished requests,
console handles and closed-page actor/shared-data entries. The candidate drops
unused capture copies, releases those owners, and asks Firefox to reclaim closed
contexts. The browser performs the normal network requests and SDK calls.

The isolated active-stream fixture completed 512 local requests while retaining
three pages. Process PSS after startup ranged from 555 to 677 MB. The primary
stream continued delivering bytes throughout. These are fixture measurements,
not six-account live memory results.

The matched comparison also completed 512 requests in each version under the
same 2 GiB container limit, with eight device contexts exercised and one primary
stream kept active. Baseline process PSS ended at 1,437,474,816 bytes with ten
pages. The repair ended at 640,508,928 bytes with three pages, a 55.4% reduction.
After warmup, repaired PSS ranged from 638,704,640 to 665,662,464 bytes. Both runs
exited successfully, and the active stream continued throughout each run.
These measurements cover one browser and local HTTP load, not production AI.
File cache from fixture preparation was reclaimed before the candidate run;
process PSS, rather than container cache accounting, is the comparison metric.

## Error-family corrections

Other candidate corrections follow the observed failure paths:

| Error family | Cause and correction |
|---|---|
| Node timeouts | Overlapping response polls and repeated large pipe-buffer copies. Keep one poll owned through its deadline and decode frames with linear copying. Report the blocked command phase. |
| Emission failures | Closed channels hid disconnects, serialization failures and task panics. Retain and publish the actual emitter or producer failure. |
| RAM admission 503 | Browser retention consumed host/cgroup headroom. Repair retention and keep the guard, reporting the controlling usage, reservation and allocation. |
| Generic admission | Preserve the first cleanup failure and distinguish account state, missing scope, occupied slots, maintenance and history-validation failure. |
| Provider 429 | Provider limits were called workspace saturation and exhausted retries could enter payload-halving retries. Classify the observed envelope and keep bounded retries on the same identity and route. |
| Slow valid responses | A throughput heuristic discarded successful output. Keep the output and the existing active-time bound. |
| Model-list 404 | Missing dialect routes. Serve the existing supported-model registry. |
| Empty/truncated responses | Missing host bodies silently became empty; terminal status errors hid the observed value. Validate the host envelope and preserve the actual parse/status failure. |
| Remote 400/502/503 | Non-JSON errors lost their HTTP status. Preserve the status and reported provider, connection or Cloudflare cause. |
| Shutdown EPIPE | Systemd signalled the host children before Rust drained. Final activation must apply mixed signal handling and verify graceful shutdown. |
| Relay warnings | A normal cache miss was logged as failure. Preserve real relay HTTP/schema errors and treat a missing account as an ordinary miss. |
| Log/storage errors | Failed writes were discarded. Report bounded I/O failures; build remotely and clean only this effort's disposable artifacts. |

The alpha.8 package later passed its qualification and isolated delivery.
Its first activation exposed the readiness defect described below. Alpha.9
includes that correction and has completed the final live activation.

The separate saved test account passed its normal authentication endpoint with
HTTP 200 and the expected identity. It is outside the twelve production
accounts. Its isolated preflight on port 8788 passed health and readiness.
The host recovered its older device inventory with the existing 600-second
hold and 24-hour reserve restriction. Neither restriction was shortened.

The UI build passed. The pinned Python run passed 289 tests; the later focused
cause checks passed 38 tests. Eight shutdown-cause checks and seven activation
and rollback checks also pass. Final Rust, package and live results are recorded below.
Earlier failed runs remain excluded from passing evidence.

Disk pressure also required an operational correction. With no local Rust
compiler running and Cargo's build lock held, cleanup of regenerable compiler
cache reclaimed 5,846,847,488 bytes. Available disk space rose from
10,835,390,464 to 16,682,237,952 bytes. Binaries and live state were preserved;
the remaining Rust builds run on the separate builder.

## Causes outside the local repair

The final cooldown review found another lost-cause path. Account cooldowns
were all presented as security refusals, including local device allowance
exhaustion. The account record now retains a sanitized cause with its deadline.
Explicit, automatic and retry admission preserve that cause. A shorter refusal
cannot replace the cause of a longer restriction, and an older successful
request cannot clear a newer cause. Existing records with no cause are reported
as unknown instead of assigning an invented security explanation.

Device diagnostics also distinguish an observed HTTP 403, recovery after an
interrupted operation, and exhaustion of the configured request allowance.
The device diagnostic map is bounded by the existing device inventory. It is
kept outside the rate-history record so the existing schema and rollback remain
compatible. A restored device deadline with no recorded cause remains unknown.
The 34 focused Python rate, recovery, capacity and cause checks passed.

The Rust regression update removes assumptions that a failed request may be
replayed on another account. Its replacement cases verify the original error,
unchanged peer accounts, released permits and preserved cleanup fences across
buffered and streaming routes. Existing tests for the old generic messages now
assert the recorded failure. Rust r4 is the qualification of this final source;
earlier failing runs are retained and are not counted as passing evidence.

A follow-up audit inspected 21,184 newer structured records. Its only new
pattern was a single incomplete-tool-call retry diagnostic, already covered
by the existing output-limit and tool-completion handling.

The matched isolation run exposed another lost-cause path: host shutdown
replaced the recorded browser-close failure with a generic summary. Shutdown
now includes each distinct recorded cause and sanitizes unexpected exceptions.
The existing close-failure case fails on the old message and passes with this
correction. All eight shutdown and error-cause checks pass, including credential
redaction and preservation of unclean history. Rust inputs are unchanged.

Framer's HTTP 403 establishes a security refusal, not its hidden risk decision.
The candidate keeps that refusal and cooldown, removes replay on another account
or device, and performs no synthetic human activity, token rewriting or forging.
The observed Kimi-pro HTTP 400 did not identify an invalid field. Browser
`NetworkError` did not identify DNS, TLS or a particular remote fault. Messages
must preserve these limits rather than invent a cause.

Earlier malformed log records and incompatible rate-history policy errors have
existing fixes in this baseline. Their preserved-history and append tests remain
relevant. Integrity verification stays strict and names differing files. Client
disconnects, explicit output/time limits, unsupported routes and wrong methods
are reported as their actual conditions, not provider or security failures.

## Readiness timeout found during activation

The alpha.8 package passed its package checks and four consecutive isolated
Responses requests. After warmup, process PSS stayed between 1.25 and 1.28 GB.
The first cold request succeeded, but its immediate successor encountered the
test container's 2 GiB limit. Idle collection recovered that memory; no OOM
event occurred. The unchanged limit admitted the four subsequent requests.

The first production activation automatically restored alpha.7 after a health
503. A readiness caller had timed out while its history check was still owned.
When the check finished, dropping its unread JSON response marked inference
cleanup as unconfirmed and closed admission. The deployment's two-second
client timeout was shorter than the readiness handler's five-second deadline.

Alpha.9 marks the completed readiness result as local completion, including
failure results. A disconnected caller still leaves the history check owned
until it finishes. The activation client allows seven seconds for that check.
Inference producers and their cleanup fences retain their existing behavior.
Both new readiness regressions and all 473 server tests passed. The new
alpha.9 package completed isolation and live activation as recorded below.
Both installed release inventories remain intact.

## Completed qualification and activation

Alpha.9 identifies clean source `132af22691dc04680f2ecadaace8e92a4d2f706e`.
Binary SHA-256: `8779007290dde036087cc27b0832ea4bbff4efd29ef08940135126d9ddaa6b35`.
Archive SHA-256: `d454ab4164bc19a6c684e74f4a8d7649b065a63b9fe142361533785081d24bb4`.

The corrected server passed 473 tests, including both readiness regressions.
Unchanged-source evidence covers 155 core, 41 store, 166 host, 89 protocol,
37 application and 440 API tests, for 1,401 passing Rust tests by composition.
The API total includes two separately run category-specific tests. The package
passed its lifecycle checks. An earlier stale executable run is excluded.

The exact staged binary returned a completed Responses HTTP 200 on port 8788
in 11.517 seconds. Both dialect model-list routes returned 200. Its exclusive
test account was outside the production account inventory. The isolated
instance was stopped before activation.

The reviewed activation procedure drained the old service and started alpha.9
at 2026-11-01 23:03:28 UTC. Health and readiness passed, followed by a completed
live Responses HTTP 200 in 18.710 seconds. PID changed from 2294848 to 2340122.
Both immutable release inventories verify. All 12 account identities, launch
arguments and rate-history schema 2 match. MemoryMax remains 10,737,418,240
bytes, MemoryHigh 9,663,676,416 bytes and MemorySwapMax 1,073,741,824 bytes.
KillMode is now mixed, so the main process owns the graceful shutdown.

Public OpenAI Responses and Anthropic Messages also returned 200 through the
public DNS address without authentication, in 1.871 and 1.138 seconds. Both
returned the expected text. Optional JWT-relay requests reported two GitHub
HTTP 401 failures; normal local minting succeeded. The relay's remote credential
was not changed. External provider refusals remain accurately reported.
The matched fixture establishes bounded isolated memory, not a production soak.
