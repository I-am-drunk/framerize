# Working-browser control boundary

The October 24 bounded log sample contains current successful browser mints
and HTTP 200 inference results. These are metadata observations, not exported
tokens or a pairing of each mint with a successful request. The service
generation remained unchanged during observation.

The installed host snapshot has SHA-256
`fd17f5a2d2547f01ccf0e62849ce4d6a67037fca7f0bd0cb3357e0636c85210b`.
Its actual path uses an existing device context and cached older wrapper.
It includes cookies, `application/json; charset=utf-8`, and `X-Requested-By`.
The experimental comparison creates a new project context, uses a newer
wrapper and separately acquires its bearers. Both existing comparison arms
omit inference cookies. That comparison does not reproduce the serving path.

`NativeHttp::with_resident_browser_headers` explicitly enables the installed
host's cookie and two fixed-header conventions. The existing authenticated
request supplies the exact API-selected cookie jar. The default remains
unchanged. Cookie and bearer header values are marked sensitive.

This option does not acquire a resident context or duplicate browser HTTP,
TLS, User-Agent, proxy fallback, referrer policy, SDK branch or bearer cache.
A native failure still cannot identify which remaining difference matters.

The private sidecar `mint` command returns only a token and can create or
navigate a page. Its normal helper can also take a non-SDK branch. It is not
an adequate fresh-token handoff. The resident operation must select an
already-open context, make one genuine wrapper call, retain exclusive account
ownership through transport settlement, and preserve account cooldown rules.
Unknown completion must retain the fence. Raw credentials stay inside that
owner. There is no established safe hot-add entry in the active process.

All 26 focused Rust cases pass, including exact loopback headers, preservation
of duplicate cookie values, invalid-header rejection, cancellation and the
existing public-contract/authentication cases. Runtime peak was 15,253,504
bytes with zero swap or limit events. Build proof starts `b49dcd3c`; test proof
starts `596382c9`. Three earlier harness failures are retained: a stale unit
name and two memory-cap refusals while rehashing historical build inputs.
The successful run reused verified build provenance and the exact test binary.
No new live mint, inference, service restart or token read was performed.

The HTTP-200 provenance patch now passes 16 inert cases. It captures the exact
pre-send tuple, publishes only after an integer 200 response completes, and
invalidates closure/replacement. Actual launch proxy and realm capability are
captured on the live account. No browser evaluation is added by this helper.
The fixture includes pending/cancelled sends, stale identities and observer
failure. The result is completed HTTP 200 evidence, not parsed success or SDK
provenance. Network-isolated peak was 31,678,464 bytes, zero swap/limit events,
and the runner was removed. Resident activity and acquisition remain pending.
