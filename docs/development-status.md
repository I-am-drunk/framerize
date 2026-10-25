# Development status

The rearchitecture is on `refactor/separate-apps-and-ci-20261025` in draft
[PR #2](https://github.com/I-am-drunk/framerize/pull/2). It is not deployed.

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

The first native Castle client primitive, `castle::v3::write_unsigned`, now
matches the pinned SDK's unsigned integer writer for 4,195 inputs. One direct
differential test covers integer-width boundaries, deterministic samples,
repeated calls and appending without changing preceding bytes. The reference
executes only source-exact `R`/`G` and six verified literal aliases from SDK
SHA256 `3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71`.
The isolated run has no network, account data or full SDK execution. Its first
attempt lacked the compiler's `cc` symlink target; adding that toolchain path
to the isolated view allowed the unchanged source comparison to pass.

This primitive is not wired into serving and makes no memory or acceptance
claim. Native CI includes `python -I -B scripts/check_v3_encoding.py`; its
hosted execution is still unverified. Next implement the typed serializer
around it, resolving the pinned source's type tags and JavaScript coercion
before comparing complete serialized arrays. Compression, collection and
the final envelope remain separate unfinished parts of the replacement.

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
