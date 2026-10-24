# What the SDK investigation can establish

The latest isolated attempt completes local preparation, sends one request,
and receives Framer's structured `request_blocked` security response. It
does not fail because the HTTP client cannot connect. It does not establish
that a model started generating. Cleanup completes; the browser service is
unchanged. The saved send-to-header interval is approximately 112 ms.

The pinned V3 return-path trace establishes that a rejected token operation
is not silently replaced by the queue's recovery result. Individual feature
operations can instead return diagnostic values that enter the normal token
serialization. A resolved token operation therefore does not establish
complete browser functionality or server acceptance. The old worker's missing
SharedWorker finding must not be applied to the later worker implementation.

The independent initialization trace closes a further question: the token
operation drains the SDK's deferred-start queue itself. It does not need a
separate page-load or warmup call to start that queued work. The queue's catch
does not manufacture a token. In the demonstrated timer-scheduling failure
path it leaves a collector pending; the current wrapper's deadline then
fails V3 and tries V2. That permanently pending path is incompatible with a
completed V3-only result. This does not exclude recoverable setup exceptions
or diagnostic fields within a completed token.

Exact V3 `3337eba7` UTF-16 spans: queue `[28284,28371)`, readiness scheduling
`[29571,29944)`, collector `[198303,198902)`, token-start call at `211468`.
Current wrapper `c32148dc` bytes `[553819,556632)` bind its deadlines,
configuration, error flag and fallback. These were read without execution.

The peer's final note adds two limits to local success checks. The inspected
configuration guard rejects a missing publishable key; it does not establish
that the key belongs to the server's Castle environment. The wrapper passes
an `onError` callback, but silence from it is not a documented guarantee of
complete signal collection. The wrapper separately enforces its own error
and deadline checks. None exposes the backend's token-validation result.

The final independent note is bound by SHA-256
`80f55999b628a5538ea97edcf25a19987f61df10992c52800cc4dd43a397ad0c`,
with verification
`3c2e66564020fa3fdca4d335ae93b47f57a72cbd8a6ffe211fc97afb0594f933`.
Its exact token prologue is UTF-16 `[211468,211498)`; configuration release
is `[292366,292384)`. Root rehashed both note and verification unchanged.

Castle's official Browser SDK documentation explicitly excludes mocked DOMs
such as JSDOM. Our worker is outside that supported environment. This is a
documented integration limitation, not identification of the exact server
rule responsible for the latest refusal.

The official integration guide distinguishes policy denial from an invalid
request token. Its example deliberately returns the same public error text
for both. Framer's server implementation is not in these downloaded client
modules, and its security envelope does not expose either underlying result.
The enum name `Castle` in our local classifier is a cooldown policy category,
not proof that we know Castle's internal decision for the request.

An exact attribution needs a correlated server-side diagnostic, or a
documented supported integration that supplies an unambiguous result. Client
decompilation can establish inputs and local failure paths; it cannot recover
an undisclosed server decision. No extra request is justified merely by a
newly generated string or another locally passing browser-API substitute.

Sources: the pinned SDK return trace `7b06782a`; saved public
[Browser SDK docs](https://docs.castle.io/docs/sdk-browser),
[integration guide](https://docs.castle.io/docs/integration-guide), and
[current model trace](native-model-contract-2026-10-24.md). The last trace
also limits comparisons with the current browser application's GPT-4.1 path.
