# Dedicated worker compatibility

The pinned Castle V3 SDK uses two dedicated workers independently of its
SharedWorker. One measures elapsed time around a debugger statement; the other
reads worker navigator metadata and optionally queries OffscreenCanvas.
Their source hashes are:

- Timing: `f9bbca7372aaea93762997eb78a3a3c3198132b22bcb787754b64059087ad4ff`
- Features: `7fbadc72ebc1e56a4274d005ec9ce05ad2435bbcc2406f8640712bbdd0987176`

The adapter runs these scripts in real Node worker threads. It permits two
workers, one empty input message per worker, and only registered local Blob
URLs. The native user agent is retained; unavailable browser metadata stays
unavailable. Worker errors expose a fixed message. Termination is joined
before the outer minter can release the token.

Eight focused cases pass, covering independent state, source and URL refusal,
early termination, active-JavaScript cancellation, private error handling,
message bounds and reply classification. One instrumented SDK run started
and joined both workers and received one feature, entropy and timing reply,
with no error or unknown replies. Its token was discarded.

The production modules use `dedicated_*` filenames under `offline_worker`.
Their separately pinned bundle passes the same eight cases and a synthetic
SDK run without diagnostic-file output. Peak was 102,232,064 bytes under the
existing 128 MiB limit, with zero swap/events and verified group teardown.
The bundle is staged; authenticated delivery remains unverified.

`qualification.json` retains the exact result hashes and the first failed
test-loader result. That failure was a missing brace in the test helper.
