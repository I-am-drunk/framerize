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

## Remaining qualification

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

Candidate alpha.8 still requires completed Rust checks, an exact-source package,
isolated upstream delivery and activation. The package source is
`72129bd3758d1fdea4d00596ddc9f1d486a144df`.
The live service has not been changed by this repair.

The separate saved test account passed its normal authentication endpoint with
HTTP 200 and the expected identity. It is outside the twelve production
accounts. Its isolated preflight on port 8788 passed health and readiness.
The host recovered its older device inventory with the existing 600-second
hold and 24-hour reserve restriction. Neither restriction was shortened.

The UI build passed. The pinned Python run passed 289 tests; the later focused
cause checks passed 38 tests. Eight shutdown-cause checks and seven activation
and rollback checks also pass. Rust checks and release packaging remain pending.
No candidate package or production activation is claimed.

Disk pressure also required an operational correction. With no local Rust
compiler running and Cargo's build lock held, cleanup of regenerable compiler
cache reclaimed 5,846,847,488 bytes. Available disk space rose from
10,835,390,464 to 16,682,237,952 bytes. Binaries and live state were preserved;
the remaining Rust builds run on the separate builder.

## Causes outside the local repair

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
