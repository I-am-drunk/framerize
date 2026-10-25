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

The retired host patchers, combined-mode checkers and their private tests are
removed. Current tests stay with the behavior they exercise.

## Validation

Native first-migration startup passes nine Rust startup cases and 15 native
Python cases. The native debug executable builds. Coverage includes holding
the profile while no listener exists, completed publication, cancellation,
failed migration, abandoned startup, rejected runtime proof, and later starts
that do not repeat expiry. Python checks preserve device identities, request
charges and unknown daily reserve restrictions. A missing primary can become
a new durable choice from the saved inventory; a known cooled primary still
refuses. All state is synthetic, with no SDK or live migration.

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

The later admission correction passes all 228 current browser-server cases
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
