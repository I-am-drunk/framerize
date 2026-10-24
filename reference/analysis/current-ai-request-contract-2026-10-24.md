# Current public AI request contract

The October 24 editor imports `chunk-OEY74USB.mjs` (`5f6b435d`) and
`chunk-5PNX2WXY.mjs` (`e909f8a5`). Public downloads match their saved SHA-256
receipts. This is static source evidence, not a successful native request.

The Responses consumer `zK` obtains a bearer through `co` (shared `ZD`),
builds headers through `lo` (`zD`), then builds the body through `BK`.
Its shared serializer `Ja` (`UD`) calls current wrapper export `b` with
the telemetry label `ai_request`, inserts `castleToken`, and adds `request`.
Routes remain `/ai/v3/responses/` and `/ai/v3/openai/responses/`.

The native client sends session and device cookies with inference. The
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

Evidence is saved in scratch `native-ai-consumer-current-r1`. Four exact
source excerpts total 13,366 bytes. REA parent `ev_ad75d7f72fc51036bebdc1e293987e3bf4a447865b6fcd7b435b0073fa3f9a58`
parsed them without execution or parse failure. Its semantic graph remains
partial/unknown with 772 unknowns; module import mappings were also checked
in the original assets. No SDK or inference request ran for this trace.
