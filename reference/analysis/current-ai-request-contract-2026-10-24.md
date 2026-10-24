# Current public AI request contract

The October 24 editor imports `chunk-OEY74USB.mjs` (`5f6b435d`) and
`chunk-5PNX2WXY.mjs` (`e909f8a5`). Public downloads match their saved SHA-256
receipts. This is static source evidence, not a successful native request.

The Responses consumer `zK` obtains a bearer through `co` (shared `ZD`),
builds headers through `lo` (`zD`), then builds the body through `BK`.
Its shared serializer `Ja` (`UD`) calls current wrapper export `b` with
the telemetry label `ai_request`, inserts `castleToken`, and adds `request`.
Routes remain `/ai/v3/responses/` and `/ai/v3/openai/responses/`.

Before correction `b57065b`, the native client sent cookies with inference. The
public client explicitly uses `credentials: include` for bearer acquisition,
but its separate inference fetch options omit credentials. The saved public
bootstrap selects `https://api.framer.com` from `https://framer.com`.
Fetch's default same-origin credential mode therefore excludes cookies on
that cross-origin AI POST. Its explicit Authorization header still travels.
The existing browser service uses a custom fetch shim with credentials
included; that shim is not evidence of the upstream application's policy.

The new request-options helper (`chunk-EIH2JWGG`, `277b8c1f`) can add a
traceparent only for two development origins, with an enabled local setting
and valid development ID. It leaves production API options unchanged.
The header helper (`chunk-7DELBDBL`, `2bb9b7ef`) conditionally adds
`X-Framer-AI-Credits-Intro-Experiment`; no evidence requires enabling it.

Cookie transmission is a concrete compatibility difference. It does not
establish the cause of the consumed security 403 or justify replaying it.
Any native correction must retain local session/device ownership checks,
cookie syntax checks, bearer acquisition, cooldowns and physical cleanup.

Correction `b57065b` now omits inference cookies and passes 39 focused cases.
The built executable `3e9398d8` was selected only on the isolated instance.
Its one live request still returned a complete Framer security 403, with
confirmed cleanup and unchanged browser service. See
`native-inference-credentials-live-20261024.json`. The compatibility difference
was real, but its correction alone did not resolve the refusal.

The same saved Responses excerpt shows that `LK` copies metadata `sessionId`
into nested `request.session_id` before serialization, replacing a supplied
payload value. The native builder previously omitted this mapping. It now
applies it only for OpenRouter Responses. Forty focused cases pass, including
an actual loopback request with a conflicting ID and other-route preservation.
This is a source correction, not an explanation for the consumed 403.

The consumed cookie-corrected trial spent approximately 6,063 ms before the
`native.http.send` event, followed by 112 ms to client-observed 403 headers.
These are saved wall-clock observations, not server or packet timings.
The response classifier requires the exact `request_blocked` reason and
security-check message. It is not merely treating every HTTP 403 as Castle.
Neither that envelope nor public SDK source exposes the rejecting server rule.

Saved dashboard evidence already confirms that the selected project is visible
to the isolated login. Repeating that check would not prove AI entitlement.
The current payload also has other differences, including optional provider
settings and outer model metadata. Their necessity is unestablished; do not
change billing categories or fabricate page activity to imitate another path.

Evidence is saved in scratch `native-ai-consumer-current-r1`. Four exact
source excerpts total 13,366 bytes. REA parent `ev_ad75d7f72fc51036bebdc1e293987e3bf4a447865b6fcd7b435b0073fa3f9a58`
parsed them without execution or parse failure. Its semantic graph remains
partial/unknown with 772 unknowns; module import mappings were also checked
in the original assets. No SDK or inference request ran for this trace.
