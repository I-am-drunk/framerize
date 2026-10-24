# Native transport response-head evidence

Branch: `goal/browserless-native-transport`, based on `71b6326`.
Source-only preparation. All builds, fixtures and runtime execution are UNRUN
in this checkout. The runtime owner retains integration and validation.

Latest owner report: R14 returned normal-browser and paired-baseline HTTP 200
with output. The explicitly configured dedicated V2 worker completed without
worker failures; native dispatch returned complete HTTP 403 JSON with the
Castle security envelope. Cleanup was confirmed and exact live command,
binary and configuration were bound. The protected service is unchanged;
no live retry is queued. This report supersedes R13 as the latest live result.

The earlier R7 browser-mint/native-HTTP control returned completed model output
(`docs/architecture.md`). Neither R7 nor R14 identifies the cause of the
isolated SDK refusal. No private captures or jars were read for this work.

`crates/framerize/src/resident_control_attempt.rs` constructs one NativeHttp
before selecting the browser or isolated SDK mint. Both paths converge on the
same sender, retained bearer and current API-selected jar. The native client
and bearer client use the captured effective proxy, including an observed
direct launch, rather than a replacement configured fallback.
`experimental_native/resident_control.rs` rechecks account generation, target,
page, device, exact bearer jar and captured egress before releasing credentials.
The SDK path also verifies the observed seed after joined worker completion.
Source binding is not a new live proof of account isolation or remote egress.

The closed credentials/include, JSON-header, cookie-order and model-serializer
findings are reused. Those implementations and fixtures were not changed.
The worker lifecycle, field decoder and runtime collector are separately owned.

Remaining source-supported transport distinctions:

- NativeHttp uses reqwest, HTTP/1 and the truthful `Framerize/0.1` user agent.
  A browser uses its own TLS and HTTP stack; its negotiated version is not
  established by this source inspection. Neither fingerprint is changed here.
- NativeHttp explicitly requests identity encoding and refuses other encodings.
  The browser fetch leaves compression negotiation and decoding to its stack.
- The browser baseline uses `cache: 'no-store'` and `redirect: 'error'`.
  NativeHttp has no browser cache and does not follow redirects, but exposes
  a received redirect status to its caller instead of a browser fetch error.
- NativeHttp supplies an origin-only Referer. The browser selects its referrer
  and automatic request metadata under browser policy and applies CORS.
- NativeHttp owns an account-specific connection pool, separate from the
  browser and bearer client pools. Equal captured proxy configuration alone
  does not attest equal remote exit IP, DNS resolution or TLS behavior.

The concrete diagnostic gap is after response headers arrive: encoding or
advertised-size rejection can occur before NativeResponse exposes its status.
The new `native.http.head` event precedes those checks and carries the existing
process-local send `invocation` plus exactly three `head` fields:
`status`, `http_version`, and `content_encoding`. HTTP versions use fixed labels
`http_09`, `http_10`, `http_11`, `http_2`, `http_3`, or `other`.
Encoding categories are `absent`, `identity`, `other`, `invalid`, or `unavailable`.
Inspection reads at most eight encoding values and 512 total value bytes.
Exceeding either bound yields `unavailable`; the original policy still runs.
Raw headers, exception strings, response content, account identifiers, proxy
addresses and request credentials are never copied into these facts.

The existing send event and counter semantics are unchanged. All dispatch,
encoding, size, cancellation, response acceptance and cleanup decisions remain
in their original code paths; the new projection does not authorize anything.

A recorded head establishes a received response head, including before a
local body-policy refusal. It does not establish body completion, model output,
token validity or joined cleanup. A missing head is inconclusive: logging is
best effort and cancellation can win before the response is observed. The
invocation is process-local, not an account or lease identity. The runtime
owner must bind it to the existing owned comparison and process evidence.
For R14, the complete 403 is already stronger response evidence. This patch
does not diagnose its cause or justify another live attempt.

Next action: the runtime owner may qualify the seven pure projection cases
under `experimental_native::native_http::transport_facts::tests` and the
existing `experimental_native::native_http::send_tests` slice in its owned
test environment. The new cases need no sockets, SDK or private inputs.
Source and staged-diff review are the only local verification. Shared status
docs, live activation and interpretation of the next comparison remain owned
by the coordinators. The full browserless goal remains active and unfulfilled.

The original retained-bearer source admitted `expiresAt > now + 30 seconds`
and returned only the token string. `ResidentBearer` did not retain expiry;
`begin_resident_mint` and `resident_token` recheck context but not bearer time.
The resident RPC budget permits 45 seconds. A hypothetical bearer admitted
with 31 seconds remaining can expire during a 32-second browser baseline
(for example, a 3-second mint and 29-second fetch, within its 30-second fetch
timeout), while a separate native mint still satisfies its freshness guard.
Thus the original minimum expiry and overall deadline were not sufficient
to establish bearer validity at native dispatch for every admitted response.
Historical evidence in `castle-contract-2026-10-05.md` reports a 600-second
AI JWT lifetime. No short-lived bearer was observed here, so this is a source
boundary case, not an explanation of R14. The source correction now requires
the initial bearer lifetime to cover the retained control deadline, without
refreshing or changing the bearer between arms.

The pure check lives in `resident_bearer_validity.rs` with six deterministic
cases in `resident_bearer_validity_tests.rs`. It was prepared dormant in
`917aeb9` and is now wired into the resident bearer acquisition only.
It takes only parsed expiry milliseconds, current wall-clock milliseconds,
and the remaining monotonic control budget. It keeps the 30-second minimum,
requires lifetime beyond the full remaining budget, and reserves one extra
millisecond for truncation of the supplied wall-clock sample. Expired,
overflowing or zero-budget inputs refuse. No credential strings are inputs.

The integration takes its monotonic remaining-budget sample before its
wall-clock sample, after the single auth response and existing cancellation
and egress checks, before publishing the bearer for the browser baseline.
It preserves one auth request, the same bearer across arms, current
account/jar/egress binding, and all existing cancellation and cleanup checks.
The private acquisition result retains expiry metadata without Debug or
serialization. Ordinary bearer callers keep their original expiry policy.
An insufficient resident lifetime returns settled preparation rejection.
This source change does not establish live bearer validity or acceptance.

All ten validity and integration cases are UNRUN here. Four added cases use
the existing bounded loopback fixture to cover short-lifetime rejection,
sufficient-lifetime retention, unchanged ordinary admission, and cancellation
after auth starts. They check the current jar and absence of another auth call.
Next action is owner qualification of the six pure checks and
`resident_control::tests::deadline_tests`, plus the existing auth/resident
regressions. No build, test, SDK, browser or live request ran in this checkout.
The integrated head-facts source is now `browserless` commit `9caa57d`;
that source integration alone does not attest compilation or fixture success.
