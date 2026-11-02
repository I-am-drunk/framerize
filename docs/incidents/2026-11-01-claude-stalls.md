# Claude stalls and hidden gateway timeouts

Work branch: `fix/claude-stalls-20261101`.
Baseline: `0.5.1-alpha.9`, source `132af22691dc04680f2ecadaace8e92a4d2f706e`.

## Direct gateway-error follow-up, 2026-11-02

Alpha.10 suppressed SDK replay when the browser hid an HTTP timeout behind
NetworkError. A directly readable HTTP 504 took the ordinary error path and
missed that protection. The new regression reproduces this on released source.
The correction applies the existing incomplete-response classification to
directly reported 502 and 504 failures during buffered adaptive generation.
The response retains the observed status and cause and disables SDK replay.
Credit and security refusals retain their existing classification.

All 117 affected Messages tests passed after a confirmed source rebuild.
The initial fixture import error, cached-executable run and transfer-permission
failure are excluded from passing evidence. The two tested source hashes match
the worktree. Version alpha.11 is being packaged; alpha.10 remains live.
No model, reasoning effort, account or custom-route policy changed.

## Alpha.10 live activation, 2026-11-02

The user explicitly authorized activation after the isolated custom request
returned HTTP 402 from its exhausted workspace. That result remains a failure.
The exact `840225c8522ccc5cdd0e09aebb3bfb39d9442d3e` package passed all nine
package checks, 481 server tests and its custom-route policy checks. Its build
ran on the existing iMac builder. Activation reused the resulting binary.

The service drained normally and started alpha.10 at 01:47:42 UTC. PID changed
from 2340122 to 2552520. Health and readiness returned 200. Live custom OpenAI
Responses completed in 24.648 seconds; a short Claude Messages request also
returned 200 and the expected text. Public HTTPS and both model-list routes
returned 200. All 12 account identities and memory limits were preserved.
Both immutable release inventories passed verification. Rollback was not needed.

Binary SHA-256: `602c5c2d6fbbf711c263989852584e62bbc0d927107a220a6659de78de0cab65`.
Archive SHA-256: `29f3ec21280d87926faa7af968d6415812510c82ca674044f9bfc49c3abef4cf`.

The live process has `FRAMERIZE_ALLOW_METERED=0`; the failing optional JWT relay
override is removed. The public proxy now forwards retry guidance. The test
instance is stopped. No account credit, identity or history was reset.
Six explicit-override controls, six existing helper tests and eight rollback
tests passed before activation. The override accepts only the recorded custom
402 with matching source and binary identity; it does not fabricate a 200.

The earlier matched browser-memory evidence remains applicable. The restarted
service's memory is not a new matched-load measurement. Long custom adaptive
generations still face Framer's upstream buffered-response time limit. The
release reports the observed cause and prevents paid fallback and unsafe replay.

## Custom-route requirement and qualification history

The user requires custom routes exclusively. The earlier long-response proof
used `FRAMERIZE_ALLOW_METERED=1` and paid upstream streaming. It consumed the
test workspace's credits and does not qualify a custom-route repair.
That isolated process was stopped while production remained on alpha.9.

The correction keeps ordinary routes on their custom transport even when an
operator permits explicit paid requests. It disables paid fallback in initial
and deferred chains and rejects metered-only models before account access.
Paid paths, header overrides and the direct agent endpoint require an explicit
operator opt-in. The deployed release leaves that opt-in disabled.

The application's Test page previously used the direct agent endpoint, whose
default model was metered. It now calls the custom Chat Completions endpoint,
shows the selected model and sends the complete conversation explicitly.
Changing accounts or starting a new chat clears that conversation. Four
web-client tests and the production UI build pass. Admission text and upstream
credit refusals remain visible instead of becoming JSON parsing errors.

Source `840225c8522ccc5cdd0e09aebb3bfb39d9442d3e` is published in PR28.
All 481 server tests pass. API evidence comprises 440 unchanged passes and
the passing rerun of a corrected fixture. That fixture had expected browser
error wording from a generic bridge error; application code did not change.
The original failed result remains recorded. The five policy regressions also
pass with metering enabled in their fixture process. Two activation-helper
tests confirm that old paid evidence cannot authorize a custom-only release.
The exact source was packaged on the iMac builder and activated as recorded above.

Long adaptive requests can still exceed the upstream buffered-response timeout.
The repair preserves the requested model and settings and reports the actual
failure without replay or paid fallback. It does not claim to remove Framer's
upstream time limit. Fresh custom-route delivery passed after authorized activation.

## Earlier diagnosis and excluded paid evidence

Eleven production Claude requests failed while using buffered upstream
generation with a 64,000-token adaptive-thinking allowance. Recent failures
took about 61 seconds and returned only `TypeError: NetworkError` to callers.
The production service remained healthy and ready.

An exclusive non-production account reproduced the failure on port 8792.
The browser's network event recorded HTTP 504 after 60.122 seconds, while
page JavaScript reported a fetch failure without an accessible HTTP status.
Preparation added latency, making the isolated request take 105.951 seconds.
A short adaptive request had completed in 16.562 seconds.

The candidate observes the existing browser request without changing it.
It matches the exact URL, method and generated turn identity, retains only
failure facts, and removes its listeners when the request finishes.
The resulting error names the observed HTTP status and browser failure.
An observed response head never becomes proof of successful completion.

When metered streaming is enabled, adaptive generation streams upstream
from its first attempt. It preserves the requested model, thinking, effort,
speed and output allowance. It does not replay incomplete adaptive work.
Automatic refund requests are removed. Provider stream errors retain their
reported type, code and message after credential filtering.

Buffered adaptive transport failures return an explicit incomplete-response
code and `x-should-retry: false`, preventing SDK replay of uncertain work.
`FRAMERIZE_ALLOW_METERED=1` remains the explicit streaming opt-in.
Long streams receive up to 840 seconds inside the host's 900-second RPC
limit, leaving time for cleanup. Buffered requests retain their prior limit.

The 51 focused Python checks pass, including cancellation, device ownership,
context retention, hidden HTTP failures and listener cleanup. An older test
expected only a generic cooldown; it now verifies the already-recorded 403
cause while retaining every identity and history-preservation assertion.

A real isolated browser correctly diagnosed 129 local HTTP 504 responses
that page JavaScript could not read. No diagnostic listeners remained.
After warmup, process PSS across the isolated container ranged from
589,548,544 to 605,046,784 bytes. This is a short local failure workload,
not a production memory soak or evidence of long AI response completion.

Source `3b3ae8231fb76a670b0719fd464c55cde9d14bad` passed 1,162 affected
Rust tests by composition: 155 core, 90 protocol, 474 server, 441 ordinary
API and two category-specific API tests. The first API run's 31 outdated
refund assertions were corrected; its failures are not counted as passes.
The exact release build and all nine package checks passed. That package
later completed the controlled long request recorded below. The credit-error
follow-up requires its own successor package.

During 48 minutes of subsequent alpha.9 production observation, six browsers
retained 17 to 18 pages and used 6.37 to 7.01 GB of cgroup memory across
96 samples. This is normal traffic, not the matched fixture workload.

The same production interval contained repeated HTTP 401 responses from the
optional JWT relay. Local token minting continued to succeed. The activation
plan removes that failed relay override only after isolated verification of
local minting. Credentials remain in their private configuration, and rollback
restores the previous environment. The inference accounts are unchanged.

This candidate retains the fixes and matched memory evidence in the
[recurring-error repair record](2026-11-01-recurring-errors.md).

The first candidate request exposed a test configuration error: its 1.5 GiB
soft memory limit throttled a roughly 1.6 GiB browser workload. The cgroup
recorded 410,604 high-limit events and 74% full memory pressure over one minute.
The test's soft limit was corrected to 2.25 GiB, matching production's 90%
ratio. Its 2.5 GiB hard cap, zero-swap limit and application admission guard
were unchanged. Browser RPC traffic resumed without replaying the request.
That attempt hit the 840-second deadline and remains a recorded failure.
Its elapsed time includes the test-induced stall and is not a latency benchmark.
A subsequent short adaptive request passed in 4.169 seconds. The controlled
long request completed in 561.606 seconds with HTTP 200 and `end_turn`.
It returned 2,475 words and reported 54,994 output tokens, including 49,914
thinking tokens. Its long duration therefore included substantial generation.

The next Claude stream and a free gpt-4.1 request both received upstream
HTTP 402. The same workspace had zero credits. The OpenAI response exposed a
separate defect: the internal `credits-exhausted` result became HTTP 500.
No further inference was attempted after confirming that refusal.

The follow-up correction keeps HTTP 402, explains the exhausted workspace
credits and disables SDK replay. Paid-route errors use the same dialect-aware
mapping. Paid streams emit actual terminal error events instead of comments
that a client can ignore. Local credit-floor refusals name the measured balance
and required minimum without suggesting another account or a free-model retry.
The new protocol regression fails on the previous source with 500 versus 402.
The follow-up passed all 92 protocol tests and all 476 server tests. Its first
server run exposed a test collector retaining bounded response buffers and an
old assertion expecting error comments. Both fixtures are corrected. A Unicode
deadline case passed alone and in the final full suite with two test threads.
Application code and test deadlines did not change for those fixture repairs.
Unchanged-source evidence covers 155 core tests and 443 API tests, for 1,166
passing Rust tests by composition. The 51 Python checks remain reusable.
The successor package was subsequently activated on the user's explicit
instruction despite exhausted test-workspace credits. The paid long result
remains preserved and excluded from custom-route acceptance.
