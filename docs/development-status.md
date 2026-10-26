# Development status

The rearchitecture is on `refactor/separate-apps-and-ci-20261025` in draft
[PR #2](https://github.com/I-am-drunk/framerize/pull/2). It is not deployed.

Nine fresh-browser response-transfer trials now pass on
`fix/browser-response-bounds-20261026`. For a completed 8 MiB local response,
128K-unit chunks reduced median transfer time from 763 ms to 434 ms, but
increased post-transfer memory. Neither chunk size established a memory win,
so the production transfer remains unchanged. The next memory experiment is
a bounded queue drained during the fetch. See the
[comparison and limits](../reference/analysis/browser-response-transfer-20261026.md).

The `fix/browser-response-bounds-20261026` branch now limits browser AI
responses to 8 MiB and keeps cancellation owned until the fetch settles.
The real-browser local HTTPS fixture also passes exact-limit delivery,
oversize refusal, cancellation and full process cleanup. Its buffered-response
snapshot used 578.818 MiB PSS including the harness and driver; this is not a
production memory-saving result and includes no SDK or real-account request.
All 17 focused Python tests pass, including 11 JavaScript fetch scenarios.
The oversize regression reproduces on the previous source. This is locally
qualified and undeployed; the native-minter/lightweight-browser connection
and its live acceptance remain unfinished. See the
[change and evidence](../reference/analysis/browser-response-bounds-20261026.md).

The `test/native-tls-account-isolation-20261026` branch passes three local
TLS tests: separate account connections and credentials, forced reconnection,
and required certificate verification. Both TLS 1.2 and TLS 1.3 were tested.
Neither resumed a session, so working session-cache isolation remains unproved.
No serving lifetime or TLS backend changed. Account transport retention
remains unimplemented and is separate from the missing browser handoff.
See the [result and limits](../reference/analysis/native-tls-isolation-20261026.md).

The `measure/shared-context-isolation-20261026` experiment now passes its
local storage, HTTPS connection, concurrent-stream and cleanup checks.
At the two-account snapshot, one browser used 551.233 MiB PSS versus
772.981 MiB for two browsers, including the harness and driver. This single
fixture comparison does not establish production savings, distinct browser
fingerprints or live acceptance. No serving path changed. See the
[recorded result and next step](../reference/analysis/shared-context-isolation-20261026.md).

## Implemented

- Separate browser and native applications in one Cargo workspace.
- Native code grouped by account, authorization, request, transport and worker.
- Browser, native and shared Python runtime packages with explicit entry points.
- Durable account history, exclusive profile ownership and joined shutdown.
- Complete archives with verification, staging and compatible-package rollback.
- A native ledger process that retains its session owner until the child exits.
- Native service state and admission without a browser bridge or browser router.
- Standalone native startup with owned sessions, ledger and a verified worker.
- An explicit legacy-history importer with atomic journal publication.
- A separate expiry transition for legacy profiles with no complete export.
- Explicit native startup integration for that first migration, before admission.
- Native Responses and Messages SSE through the existing request owner.

The retired host patchers, combined-mode checkers and their private tests are
removed. Current tests stay with the behavior they exercise.

## Validation

The connected worker now has a measured memory baseline. One isolated
synthetic operation sampled 118.153 MiB PSS for the JavaScript worker and
121.395 MiB for the Rust owner plus worker. It completed in 2.225 seconds
with joined cleanup; three accounting tests pass. These are sampled lower
bounds, not live inference measurements or demonstrated savings. The
remaining SDK/JSDOM process accounts for most of the observed footprint.
See [the measurement and limits](../reference/analysis/native-worker-memory.md).

Actual SharedWorker and network receiver results now reach Rust, whose
reductions supply all five supported fields to the encoder. Their duplicate
SDK reductions are removed; the SDK retains the genuine network timing path.
A fresh comparison matches all 765 positions, 5,412 serialized bytes and the
original 5,297-byte output, with eleven damage/mismatch checks passing.
Eighteen JavaScript and 91 Rust checks pass. After removing the network
reductions, seven affected JavaScript and five Rust checks pass again, and
the production loader completes a synthetic operation in 2.02 seconds with
the worker joined and absent. Collection still uses SDK/JSDOM; live acceptance
and request memory savings remain unverified. This timing is not a speedup claim.
See [the contract and evidence](../reference/analysis/native-collection-handoff.md#qualification).

The fresh collection frame now passes a comparison through the production
Rust decoder and encoder. All 765 positions reproduce the same operation's
5,411 serialized bytes and 5,297-byte output at its recorded clock. Seven
damage/mismatch checks pass, and the passing capture has confirmed child
exit and no remaining descendants. An earlier outer cleanup-check failure
is retained separately. The existing snapshot checker also accepts the new
capture format. See [the evidence and limits](../reference/analysis/native-collection-handoff.md#fresh-frame-comparison).

Fresh SDK collection now reaches the native encoder through the private
worker pipe. One isolated synthetic operation completed in 1.94 seconds,
with worker cleanup joined and its output discarded. Fourteen JavaScript
checks and 89 focused Rust checks pass. Collection still uses
SDK/JSDOM; live acceptance and request memory savings remain unverified.
See [the handoff and qualification](../reference/analysis/native-collection-handoff.md).

The native encoder now matches one isolated SDK operation using all 765
actual input positions captured before serialization. It reproduces 5,412
serialized bytes and the 5,297-byte final output, including native rebuilding
of the five supported collector fields. Three capture checks also pass.
This establishes the saved direct-input comparison. The new serving handoff
uses SDK collection with native encoding; complete native collection is next.
See [the observation and limits](../reference/analysis/castle-v3-sdk-snapshot.md).

Shared-worker cleanup now waits for actual exit even when stopping fails.
Seven inert checks pass on Node 24.13.0; the three stop-outcome cases reproduce
premature cleanup completion on the previous source. This is adapter cleanup
qualification only. No SDK operation or deployment ran for this change.

Native network sorting now supplies an explicit-locale comparator instead of
requiring JavaScript to precompute the order. Four boundary tests, 133,128
ordering pairs across 18 profiles and 30 complete record comparisons pass on
Node 20.20.0 and the CI runtime Node 24.13.0. Lone surrogates and unknown profiles
are refused. This is collection support, not a completed collector or live
replacement. See [the result and limits](../reference/analysis/native-network-collation.md).

The native network assembler now also exposes the explicit unavailable
record. Its complete serialized output matches the pinned SDK's `xP()`
helper and differs from a normal empty observation. All 32 record comparisons
and the non-scalar refusal check pass in three isolated tests. The existing
CI comparison includes this case. No new SDK startup, network collection or
live request was performed; runtime availability detection remains external.
The saved synthetic SDK capture also passes with its entire unavailable
network record rebuilt natively before applying the consumers. Its 5,412
serialized bytes and 5,297-byte final output remain identical. One native
test passed; the SDK capture was reused without starting another operation.

The native collector-result integration writes SharedWorker position 242
and all four network positions: 80, 469, 473 and 659, in source order.
The related-address and protocol assignments pass 202 original-helper
comparisons; 14 network-assignment comparisons preserve earlier writes on later
failure. The existing 101 hash, 57 address and eight assignment comparisons
also pass. These and refusal/preservation checks run as 25 native tests.
`apply_network_result` now performs the four ordered assignments from one
consumed observation, without rebuilding or cloning its record. It refuses
missing destinations and retains the completed prefix on a later failure.
Reusing the saved synthetic SDK capture, a further native test clears and
rebuilds all five positions in source order, using the native unavailable
network record. Its complete 5,412-byte
serialization and final 5,297-byte output match the same recorded operation.
No new SDK run was needed. Generic accessors and non-string field coercion,
native collection for other fields and live acceptance remain unverified.
See the [source and result note](../reference/analysis/castle-v3-collected-inputs.md).

The [isolated SDK snapshot comparison](../reference/analysis/castle-v3-sdk-snapshot.md)
now passes on one actual instrumented SDK operation with synthetic context.
All 765 positions (787 total values) round-trip to the same 5,411 serialized
bytes. Native final encoding matches the SDK's 5,293-byte output by SHA-256
and length, using the same recorded timestamp. The initial launcher omitted
the syscall filter and correctly failed before SDK evaluation; the corrected
launcher reuses the production filter and passes its denial checks. The
runtime guard is unchanged. Both runs joined, with no account or upstream
request. This is post-collection parity, not a browser or RAM-saving result.
Next bind the actual collector semantics and ordering to the native inputs.

Four native Castle client primitives now match the pinned SDK's unsigned,
signed and fixed-width integer writers and its UTF-16 string writer. Four
direct differential tests compare 12,630 inputs: 4,195 unsigned, 4,219 signed,
4,195 fixed-width and 21 string inputs. They cover integer-width boundaries,
deterministic samples, repeated calls and appending without changing preceding
bytes. Strings include length-prefix boundaries, embedded NULs, surrogate
pairs and unpaired surrogates. The reference executes only source-exact helper
functions and verified aliases from SDK
SHA256 `3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71`.
The isolated run has no network, account data or full SDK execution. Its first
attempt lacked the compiler's `cc` symlink target; adding that toolchain path
to the isolated view allowed the unchanged source comparison to pass.

The typed Rust serializer built on these primitives passes 528 complete-array
comparisons against the original serializer and its source-exact helpers.
The cases cover null/undefined, booleans, numeric boundaries, nonfinite values,
negative zero, float bytes, canonical Base64, rejected Base64 padding, Latin-1,
UTF-16, array-length boundaries and nested ordering. Repeated serialization
produces the same bytes. No full SDK startup or live account was involved.

The original explicit JSON fallback remains available for caller-evaluated
custom objects. Native plain-object conversion now also passes 932 complete
array comparisons against the original serializer. These cover property
ordering and duplicate keys, undefined/function/symbol omission, nested arrays
and objects, Unicode escaping, and 867 numeric values including exponent
boundaries and deterministic samples. Failed JSON conversion discards the
whole object result, preserving adjacent values. Custom getters and `toJSON`
evaluation remain caller-owned; this does not implement arbitrary JavaScript.
The formatter is the locked `ryu-js` 1.0.3 dependency. The first native compile
needed an explicit slice type; the corrected implementation passes.
The first reference extraction and generated Rust fixture needed boundary and
type-annotation fixes; both failures are retained. The final comparison passes.

The native compressor now matches the pinned SDK fallback on 71 distinct raw
inputs and 14 complete serialization/compression cases. Both differential tests
pass, including empty input, repeated bytes, varied symbol frequencies, long
inputs, match-length boundaries and the 1,024-byte search-window boundary.
Every reference stream independently decompresses to the original bytes with
zlib. The fallback is deterministic; the SDK's optional platform compressor
may produce different valid bytes and has not been equated with it.

The final envelope now passes 135 complete-output comparisons against the
original function, using synthetic payloads and explicit timestamps. Fourteen
additional cases match the combined serialization, compression and envelope
sequence. A third test rejects timestamps outside JavaScript Date's range.
The comparisons include empty, odd/even and long payloads, repeated calls,
negative times, decimal-width boundaries and both date-range endpoints.
The shared reference extraction changed; its 528 serializer comparisons pass
again. These are isolated offline checks, not full SDK or service execution.

The V3 worker now uses native encoding; collection still uses SDK/JSDOM.
These comparisons make no memory or acceptance claim.
Native CI includes `scripts/check_v3_encoding.py`,
`scripts/check_v3_serialization.py`, `scripts/check_v3_objects.py`,
`scripts/check_v3_worker_result.py`,
`scripts/check_v3_network_result.py`,
`scripts/check_v3_hash.py`,
`scripts/check_v3_network_record.py`,
`scripts/check_v3_network_events.py`,
`scripts/check_v3_collected.py`,
`scripts/check_v3_deflate.py` and `scripts/check_v3_envelope.py`;
hosted execution remains unverified. All prior encoding comparisons also pass
with the shared fixture builder and new native object module.
Next bind the actual collector values and any custom-object conversions to
this native representation. Device collection, complete client interoperability
and measured RAM/latency remain unfinished.

The native `encode_collected` entry point now combines serialization,
fallback compression and final encoding from a supplied complete value array.
It passes 17 comparisons against the original SDK stages, including three
765-position synthetic arrays; a second test preserves invalid-clock refusal.
Both tests pass with zero skips in isolation. This is a complete post-collection
encoder, not a complete collector or a serving integration. The arrays in the
comparison are synthetic and do not establish genuine device observations.

The [stock shared-worker schema](../reference/analysis/castle-v3-shared-worker-schema.md)
is now decoded from the pinned source. Its previously untyped element 10
comes from `userAgentData.uaFullVersion`, with string markers on unavailable
or failed paths. This does not establish a stock object producer there.
Only the pure string decoder ran; worker execution and live selection were
not tested. Its native `shared_worker_version` reducer now passes 100
comparisons against the original `Qon` followed by serialization. These cover
the eight known markers, unknown strings, strict numeric tag `2`, missing
element 10, JavaScript truthiness and selection without reinterpreting the
result as another marker. A second test rejects already-normalized selected
values whose original truthiness is unavailable. Both tests pass, zero skips,
inside the existing 512 MiB isolated harness. Earlier component results are
retained; the shared reference's only existing-code change exports its decoder.

The [field-38 callback trace](../reference/analysis/castle-v3-network-field38.md)
also establishes a string on ordinary stable RTCIceCandidate paths. Empty,
timeout and selected-address values remain distinct; `krn` substitutes a
marker for an empty field before writing `Q[473]`. No stock non-array object
producer was found on this path. The native `network_address` reducer now
passes 57 comparisons, including five original missing-record errors, plus
nine normalized-input refusals. Both tests pass in isolation. It retains the
original distinction between an absent record and an absent field, and assumes
the built-in getter rather than a replaced JavaScript binding. The independent
metrics-hash input trace is complete. Actual device collection and service
acceptance remain unfinished.

The native JSON metrics hash passes 163 comparisons against the original
`UP` helper, including nine refused conversions. The cases cover property
order, duplicate keys, omitted values, nested arrays and objects, Unicode,
numeric formatting and hexadecimal padding. Repeated calls agree. The first
run reached the generated fixture's compiler recursion limit; the corrected
fixture passes with unchanged native implementation.

The network-record assembler now passes 31 complete-record comparisons
against the pinned SDK's original finalizer. These cover field positions,
selection before sorting, candidate/history caps, duplicates, scalar
conversion, Unicode, summary keys and literal separators. A separate test
checks non-scalar refusal. Both tests passed with zero skips in the isolated
512 MiB harness. The resulting sparse record and its embedded metrics hash
match after serialization. The comparison supplies the original runtime's
signature order; it does not implement or validate a native locale comparator.
Candidates and histories enter after event reduction. No service or account
was used for these tests.

Native `NetworkEvents` now implements that preceding reduction from supplied
typed RTC properties, candidate text, SDP and state events. It passes 56
complete event-sequence comparisons against the original source, plus 32
address-class and 13 numeric-port comparisons, in 57 tests with zero skips.
The cases include branch selection, signature collisions, caps after
deduplication, event ordering, fallback syntax, Unicode and nonfinite ports.
The comparison caught and corrected a native trailing-newline classification
error. Two earlier reference-extraction failures are retained separately.
The isolated run uses the existing 512 MiB harness and Node Unicode 16.0.
No new runtime dependency was added. Generic JavaScript getters/coercions
are outside the typed observation input; Unicode tables remain a platform
boundary. Next connect genuine collection inputs and their finalization order.
Real collection, locale collation, serving integration, acceptance and measured
memory/latency remain unfinished.

The dedicated-worker cleanup correction from `759ef0d1` is integrated on
the reorganized `24ebbdb` base. A focused regression fails on the old pool:
it reports a stop failure before the workers have exited. All eight inert
cleanup cases pass with the correction, with no skipped cases, inside an
isolated Node 20.20.0 run. They cover failed and successful stops, pending
exits, discarded stop promises, source preparation and already-exited workers.
The native CI job now runs this selection. No SDK, live account or service
was used; the matching package result is recorded below.

The G3 correction from `88db846` is integrated over `d760f0a`. A later worker handoff refusal now
retains completed minter metadata. Rust retains a validated typed summary and
emits only fixed labels, booleans, bounded counters and finite timings. Early
errors remain minimal; malformed reports remain generic worker failures.

Failure-only `token_pipe_write_attempted` marks entry into the token writer;
`token_pipe_write_completed` means it returned successfully. A failed writer
may have written partial bytes. Neither flag means an age-qualified
`token_handoff`, parent receipt, acceptance or joined cleanup. Unmeasured or
invalid handoff age is null. The success schema and parent guards are unchanged.

The old JavaScript entrypoint fails the report-retention regression. Its
replacement passes all eight handoff cases, and the Rust parser passes all
22 failure cases. Twelve additional Rust checks pass for HTTP/SSE behavior,
request correlation, cancellation and refusal without a device. No selected
case was ignored. The native CI job includes the JavaScript handoff selection.

Worker failures and request completion now share an internally generated
per-request UUID. An isolated regression checks actual emitted records from
two synthetic requests through the same account: each failure pairs with its
own completion and contains no account identifier. The checks use inert
workers and synthetic state, not the vendor SDK or a live account.
The earlier eight pool-cleanup passes remain valid and were not repeated.
The matching optimized package is now built and checked as recorded below.
The historical HTTP 403 still has no established policy cause.

Native requests now retain one `offline.request.finished` diagnostic after
their owned cleanup settles, even if the JSON or SSE consumer disconnects.
It uses the same safe outcome fields and generated request ID as
`offline.complete`, without claiming delivery or parent-task join. Both
recorders contain logging panics so diagnostics cannot change the outcome.
All seven isolated disconnect/logging cases pass. The old implementation
fails the dropped-JSON case with zero records instead of one. The 41 service
cases and worker-correlation case passed against unchanged recorder sources.
The corrected qualification rebuilt the native test crate after identifying
an old baseline executable reused from a shared Cargo target directory.
These are synthetic checks; the installed service was not changed.

The optimized native release from `04a5fc2` includes the cleanup and diagnostic
changes and passes all eight package lifecycle checks using its bundled worker.
The complete archive is 50,420,493 bytes, with SHA256
`1b2ba1c45db56d5ff1d1b11c458d644ee210295676b44347bf37b7c2df80b8ad`.
Its executable matches the release build, and its worker entrypoint and pool
match the qualified source. The checks cover archive/runtime verification,
startup, invalid requests, exclusive profile ownership, SIGTERM, SIGINT and
profile reuse. Build plus packaging and checks took 72.984 seconds with cached
dependencies. The isolated unit exited and released its execution lock.
No SDK, live account or installed service was used. This is a local package
result. GitHub Actions run `40215277433` ended in `startup_failure` before any
job ran; the previously recorded hosted billing restriction is unresolved.
Next: bind the remaining native acceptance observations to this package and
its request IDs. Existing-account ownership and real delivery remain unverified.

The previous optimized native release from `3c5a999` also passed all eight
package lifecycle checks using its bundled worker configuration. The
complete archive is 50,406,919 bytes, with SHA256
`e780b8d80709f0c064c53cefa90edc0c010a4caa2d9c0d351c57db421eaf109e`.
Its executable matches the release build. These are artifact sizes, not
runtime memory measurements. No SDK or live account was exercised by these
checks. Existing-account migration and native upstream delivery remain open.

Open browser streams can now wait for local account capacity before generated
output. The cumulative wait is bounded to 600 seconds, releases model and
resident permits while sleeping, preserves real cooldowns and cancels on
disconnect. Initial HTTP admission, explicit accounts, prior upstream refusals
and cleanup rules keep their existing policy.

All five source deltas match the qualified `bc47d3d` browser repair. The
reorganized branch passes all 236 server tests with zero ignored; the narrow
repair branch passes 237. Capacity recovery, heartbeat, disconnect and the
full virtual-time budget are covered. The narrow browser package was activated
by an authorized restart at 16:18:57 UTC. Its executable and health PID match;
through 16:21:18 it recorded 152 terminal reply chains and no chain errors.
This branch remains undeployed, and the initial traffic window does not prove
every affected T3 continuation recovered or that a capacity wait was exercised.

The current `c78eeeb` native development archive includes the final-fragment
status correction and latest shared server fixes. Its executable matches the
newly built source, and all eight package lifecycle checks pass with the
unchanged bundled worker configuration. Archive SHA256 is
`605b9ee7926c33c7c09b2944f6174b45a4e6649052f36620b711955b2c57ff31`.
No SDK or live account was used. This closes the gap between current source
and the older packaged native executable; real native delivery is unverified.

The latest browser correction also covers account cooldown after opening a
stream and a replacement account refusing admission while another peer is
ready. Both defects were reproduced before their fixes on the narrow repair
branch, where all 234 server checks pass. The equivalent changes here pass
all 233 server checks, including finite selection and unknown-cleanup refusal.
Explicit account choices, real cooldowns and inference retry limits remain
unchanged. This branch is not deployed; these checks do not prove that all
reported T3 continuation failures are resolved.

A native request regression reproduced completed continuation text retaining
the first fragment's incomplete item status. The shared merge now takes the
latest supplied status while retaining the first identity and combined text.
All 44 native request checks and 15 browser Responses chain checks pass,
including the existing cooldown-continuation case; the native build passes.
This change is not deployed and does not establish live cooldown recovery.

Native streaming now supports explicitly bounded text continuation, with a
separate caller-supplied authorization context for each further chunk. Every
chunk requests fresh credentials through the existing device/rate owner.
The complete-JSON endpoint remains one-attempt, and refusals are not retried.
A regression reproduced output usage reporting only the last chunk; the
corrected response reports cumulative output and reasoning counts.
All 131 selected native request, authorization and service checks pass,
including eight new continuation cases. These cover both formats, context
binding, fresh authorization, cancellation, later refusal, attempt bounds,
incomplete output and withholding unaccepted HTTP output. The native build
passes. This is synthetic qualification; live native delivery is unverified.
The clean `f70e54e` native development archive also passes all eight package
lifecycle checks with the unchanged bundled worker. Its executable matches
the qualified build. The package is not installed in the live browser service.

The existing temporary browser archive checks are now consolidated in
`scripts/smoke_browser_package.py` and wired into the browser CI job. The
clean `4243da2` development archive passes all nine lifecycle checks, including
an actual request for the bundled UI, profile exclusion, both shutdown signals
and later profile reuse. Its executable matches the qualified browser build.
This uses empty temporary accounts and runs no browser, SDK or inference.
The archive is retained for development, not installed in the live service.

Both browser cooldown repairs are integrated into the reorganized application.
A queued unbound request can release a newly cooled account and select an
eligible peer before sending output. A partly delivered reply can wait for
the reported local cooldown on the same account and retry only the unfinished
chunk. Waiting has a separate cumulative ten-minute limit; cancellation,
delivered output, usage and transport-failure timing are preserved.
All 231 current browser-server cases and 27 selected API cases pass, with no
ignored cases. The browser development build passes. These are isolated
synthetic checks. The user's 2026-10-25 15:23 UTC restart activated the separate
narrow repair, confirmed by the running executable's hash. The recorded traffic
window through 15:37 UTC had 857 successful terminal chains and 19 local
cooldown/account reselections. This does not prove every T3 thread recovered or
that the mid-reply wait was exercised. Subsequent T3 errors at 15:40, 15:43 and
15:48 confirm the cooldown loop is still unresolved. This rewrite is not
deployed, and no account cooldown or history was reset.

Native startup now rejects unknown selected accounts before starting legacy
migration or the ledger process. Previously it could complete the ten-minute
expiry first and only then reject the selection. A regression reproduces that
ordering defect. All 11 focused startup cases pass with the correction,
including a mixed valid/invalid selection and an account removed during the
migration wait. The post-wait check runs before listener admission. These are
synthetic profile tests; the native development build also passes. No live
account or service has been migrated.
The clean `96a79c3` development archive passes all eight package lifecycle
checks with the unchanged bundled worker: archive/runtime verification,
startup, invalid-request refusal, profile exclusion, both shutdown signals
and later profile reuse. The archive's executable matches the qualified build.
This package is not deployed and these checks do not run the SDK.

The native `/api/offline/stream` endpoint now sends opening frames, heartbeats
and complete output items through the same bounded request as the JSON
endpoint. Terminal success waits for account and device cleanup. Disconnects
cancel that request while its existing owners finish cleanup. Incomplete tool
output and unaccepted HTTP metadata cannot publish generated items.

The native build passes. There are 74 passing focused service/request cases:
70 retained unchanged results and four affected cases rerun after the final
metadata check. Eight new cases cover both stream formats, response identity,
output before cleanup, failure instead of false success, disconnects before
and after body polling, buffered upstream timing and rejected output. These
use synthetic state and fixture-owned loopback, with no SDK or live account.
Upstream remains buffered and requests remain limited to one attempt. This
does not establish live streaming latency or native upstream acceptance.
The complete `3205333` development archive also passes all eight relocated
package lifecycle checks with its bundled worker, including both shutdown
signals and subsequent profile reuse. It is retained for development and is
not installed in the running browser service.

The reorganized branch now includes the follow-up browser admission repair.
All 139 core and 229 server cases pass on this composition. A matching Busy
close receipt retains the occupied browser slot without declaring a new
unfinished close. Completed status checks followed by a local account fence
also retain confirmed cleanup. Unknown cleanup and mismatched identities keep
their restrictions. This is source qualification, not a new service deployment
or proof that every historical lockout trigger has been resolved.

Native first-migration startup passes nine Rust startup cases and 15 native
Python cases. The native debug executable builds. Coverage includes holding
the profile while no listener exists, completed publication, cancellation,
failed migration, abandoned startup, rejected runtime proof, and later starts
that do not repeat expiry. Python checks preserve device identities, request
charges and unknown daily reserve restrictions. A missing primary can become
a new durable choice from the saved inventory; a known cooled primary still
refuses. All state is synthetic, with no SDK or live migration.
The complete `1b9a7fe` development archive also passes its eight lifecycle
checks: archive/runtime validation, startup, invalid-request refusal, profile
exclusion, SIGTERM, Ctrl-C and profile reuse. It is not installed in the running
service. Real browserless generation remains unverified.
The same package also passes all eight lifecycle checks with the original
V3 worker configuration. The portable worker builder now carries all 1,259
original inputs: 1,258 are byte-identical, and only the location declarations
changed. Node, SDK assets, support modules and npm dependencies are bundled.
Three Rust path-resolution cases and seven package/staging cases pass; the
native executable rebuilds and the packaged software pin checks pass.
The complete `fe306ec` development archive passes all eight lifecycle checks
after staging elsewhere, using its default bundled worker. Startup, profile
exclusion, invalid-request refusal, both signal shutdowns and profile reuse
pass with synthetic state. All six pinned public asset downloads also match.
No SDK execution or upstream acceptance is established by these results.

The portable V3 worker has now also executed the unchanged real SDK with
synthetic context, inside a private network/PID namespace with homes hidden,
no new privileges and a 128 MiB cgroup limit. It generated one token, handed
it to the Rust owner, joined the worker and discarded the token. The run took
2.35 seconds, including 544 ms inside the SDK method. No upstream request or
real account was involved, so live native acceptance remains unverified.
The existing worker measurement is now a normal Cargo example using the same
configuration loader as the native service, with its duplicate parser removed.

The expiry, importer, journal, rate and runtime-owner suites pass 41 focused
Python cases. Thirteen expiry cases cover actual deadline boundaries, unknown
daily reserve use, account isolation, changed owners, clock discontinuities,
existing history, failed writes and inventory changes. These use synthetic
state and clocks. Migration is never selected implicitly, and no live state
has been migrated. The complete-snapshot importer still requires a final
source export. Existing native and browser package results are retained.

The complete browser and native development archives at `2410dac` pass
verification and staging. Browser startup, signal shutdown and profile
exclusion pass. The native archive starts and joins its packaged ledger.

The native admission separation passes all 24 current native service cases and
all 226 remaining browser-server cases. Both debug executables build. The first
compile identified a private method moved across the crate boundary; exposing
that unchanged registration method resolved it.

The prior 288 native results and 110 Python results cover unchanged code.
Four obsolete combined-mode native cases were replaced by four admission
cases; two independent browser health cases were retained. Removed tests do
not count as validation of the new structure. No live delivery is claimed.

The earlier admission correction passed all 228 then-current browser-server cases
and all 34 browser application cases. Its new account-switching regression
fails on the previous implementation and passes with the correction, for
streaming/nonstreaming and consumed/dropped responses. A completed negative
status read keeps the affected account fenced while other accounts can serve.
Failed reads, timeouts and unconfirmed closes retain their refusal behavior.
Health reports 503 when the global operation gate is sealed or unconfirmed.

The 25 October outage was a persistent global admission refusal, not a process
crash. The exact underlying browser check that triggered it was not logged;
the regression establishes a source defect, not that historical trigger.
The narrow repair tracked in PR #3 is running on the original source base; this
rewrite remains undeployed. Health reporting alone does not implement automatic
recovery, which remains unfinished.

Standalone native startup passes 29 service cases, including its admission-aware
health endpoint. Both debug applications build. Six release tests pass, including
refusal of a native archive missing its runtime. Separate child process groups
pass all 64 worker-process and seven ledger-process cases. The `dd2b92d` archive
passes the actual native serve command, profile contention, invalid-request
refusal, SIGTERM, Ctrl-C and profile reuse with synthetic state. It runs no SDK.

## Remaining work

The native CLI now serves ledger-backed V3 with an existing migrated profile.
Real upstream delivery is unverified. The service now watches the ledger's
process exit and stops serving when that owner is lost. All five startup and
seven ledger-process cases pass, including forced helper exit, listener closure
and profile reuse; the native executable rebuilt successfully. The updated
`251eba5` native archive also passes all eight package lifecycle checks.
V2 requires its authenticated SDK proof; no fallback substitutes for it,
and this CLI does not enable V2 serving.

First migration of legacy account history remains unresolved. The working
browser service and its account state have not been changed by this rewrite.
The explicit pre-admission migration stage now holds session ownership and
joins cancellation. First live use still needs the actual previous service
identity and an exclusive idle interval. This does not authorize interrupting
the current service or guessing its history. Native live delivery remains
unverified; the new startup path alone does not establish SDK acceptance.

The requested alternate GitHub account (`diddletime`) and private build fork
are configured. Both available accounts have Actions billing restrictions;
no hosted build has succeeded.

See [architecture](architecture.md), [packages](releases.md), and the
[collected milestone record](history/rearchitecture-progress-20261025.md).
