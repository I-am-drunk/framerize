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
The candidate still needs complete Rust and real-request qualification.
