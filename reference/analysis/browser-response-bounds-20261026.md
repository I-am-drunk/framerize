# Browser response bounds

Branch: `fix/browser-response-bounds-20261026`.
Base: `36e8a624094d779a9209945d96c4e719a5163a2a`.

The existing browser AI request path now limits response payloads to 8 MiB.
It counts bytes delivered by the body stream, rather than trusting the
Content-Length header. Oversized output aborts the request and returns the
existing body-failure result with the observed HTTP status. Partial output
is never returned as a successful response.

Incremental UTF-8 decoding preserves split characters, BOM handling and
replacement characters. Small chunks are grouped before final assembly.
This bounds retained payload, not browser overhead or total process memory.

Cancellation now awaits the original fetch and body-reader settlement. The
existing host RPC owner retains the account while that wait is unfinished,
including when the caller cancels its own cleanup wait. The abandoned-job
timer also preserves this joinable operation until it settles.

Local qualification on 2026-10-26 passed 17 Python tests across `test_fetch`,
`test_activity` and `test_post`. The fetch wrapper ran all 11 JavaScript cases.
They cover exact/oversized limits, UTF-8 boundaries, empty responses, failure
phases, delayed cleanup, cancellation and the abandoned-job timer.

The new oversize regression fails on the exact previous script: it returns
HTTP 200 for an oversized response. Both isolated test units exited cleanly.

These fixtures use inert browser ports and standard JavaScript streams.
They run with private networking and private account paths masked. No browser,
SDK, account, live request or service deployment ran for this change.

The native-minter-to-lightweight-browser handoff remains unimplemented.
This fix does not establish live delivery, account unlinkability or measured
memory savings for that combination. Token generation is unchanged.
