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

Owner record `bff2afc` now reports successful production/test compilation,
all seven projection cases and all seven existing send cases. The test run
peaked at 14,729,216 bytes, with no swap/limit events and joined process groups.
These are reused owner results; no tests were executed in this checkout.
Source and staged-diff review remain the only local verification. Shared status
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
Owner receipt `cf723fc9` reports successful production/test API compilation
and all 28 focused cases: six pure validity, four deadline integration and
18 existing resident cases. No build, test, SDK, browser or live request ran here.
The owner reports test peak memory of 16,629,760 bytes, compile peaks of
294,805,504 and 364,277,760 bytes, zero swap/events and the qualification unit
absent after completion. These are owner results, not local measurements.
The source commits `917aeb9` and `ad31379` are integrated on `browserless`
as `3834b71` and `3b13923`, respectively. Qualification closes the pending
fixture work; it does not establish a fresh accepted browserless inference.
The owner's next stated action is source publication, with no deployment or
live retry from this change.
The integrated head-facts source is `browserless` commit `9caa57d`;
its subsequent owner qualification is recorded above.

Source lineage comparison used the October 24 R7 result landmark
`825ec7bc125a13b22aed93c92048d1aa0d5dedc4` and R14 result landmark
`9e7630db48e15ebb6922ca980260c3aa81ec2da2`. Other historical runs reuse
these R numbers, so their names alone are not adequate comparison keys.
`git diff --name-status` returns no changes for these seven tracked paths:

- `crates/framer-api/src/experimental_native/native_http.rs`
- `crates/framer-api/src/experimental_native/native_send_evidence.rs`
- `crates/framer-api/src/transport.rs`
- `crates/framer-api/Cargo.toml`
- `crates/framerize/Cargo.toml`
- `Cargo.toml`
- `Cargo.lock`

The shared `native_http.rs` blob is `bd05ef943fb6fb851afe36b3de76b301038602aa`;
the shared lockfile blob is `c18b71f2c9af76718bb9c7e9d853c5da601271ad`.
The resident attempt delta adds explicit SDK selection, checked SDK context
and its freshness origin. Its native `send` and `collect` functions are
unchanged. The changed cookie-header source remains subject to the existing
private context validation; this finding does not reopen cookie ordering.

This establishes unchanged tracked sender code and dependency declarations
between the two documentation landmarks. It does not bind either frozen
executable to that exact checkout, prove identical wire bytes, or establish
the same TLS, DNS, egress or backend state. Those require the runtime owner's
existing build and comparison evidence. No fresh live comparison is implied.
The lineage note `3964361` is integrated on `browserless` as `31d31a5`.
