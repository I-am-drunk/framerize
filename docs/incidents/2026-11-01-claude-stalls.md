# Claude stalls and hidden gateway timeouts

Work branch: `fix/claude-stalls-20261101`.
Baseline: `0.5.1-alpha.9`, source `132af22691dc04680f2ecadaace8e92a4d2f706e`.

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
The exact release build and all nine package checks passed. Isolated
long-request qualification remains pending.

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
A subsequent short adaptive request passed in 4.169 seconds. A new bounded
long request is running under the corrected limits, without further throttling.
