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
