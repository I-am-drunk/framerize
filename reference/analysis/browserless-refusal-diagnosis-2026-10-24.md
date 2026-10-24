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
