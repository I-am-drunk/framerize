# Development status

The reorganization is on `refactor/separate-apps-and-ci-20261025`, in draft
[PR #2](https://github.com/I-am-drunk/framerize/pull/2). It has not been deployed.

Completed source changes:
- Separate browser and native applications in one workspace. The browser
  executable does not depend on the native implementation.
- Native code grouped by account, authorization, request, transport and worker.
- A Python runtime package with account, browser, resources and wire modules.
  Runtime imports do not launch browsers.
- Complete executable, host and UI archives, checksum manifests, and a staging
  command that leaves the running service untouched.
- Explicit host/UI development overrides. Installed applications select the
  assets next to their canonical executable.
- Explicit concurrency-limit persistence at application startup, atomic file
  replacement, and restoration of the actual permit count.
- Browser-only Rust and Python hosts. Removed the observation/comparison
  adapters, their dispatch and native startup overrides. Account RPC ownership
  remains in the browser runtime, independent of native authority.
- Removed the retired comparison/resident fixture stack and its custom
  qualification scripts. Ordinary lifecycle and fetch checks now import the
  runtime that the browser archive ships.

Validation collected locally:
- The workspace all-target check passed at the application split milestone.
- Current browser server: 251 tests pass. Host: 72 tests pass. Browser CLI: 33
  tests pass. Gate selection: 43 tests pass, including three new persistence tests.
- Python runtime and release tooling: 99 fixture tests pass across lifecycle, imports, browser setup,
  packaging, rate persistence, request ownership and safe diagnostics. The fetch fixture
  runs the packaged JavaScript using an inert Node harness.
- The TypeScript/Vite UI build passes. Both local development archives pass
  checksum verification, staging and executable startup; the packaged browser
  host starts and exits on EOF using fresh temporary state without a browser.
- After the host cut, a fresh browser development archive passes the same checks
  and excludes observation/native packages. Debug builds remain explicitly
  marked as development even from a clean checkout; its regression test passes.
  Packaging omits empty directories left by removed runtime modules.
- Native: 302 accepted cases across the collected worker and request selections.
  The eight stale request-envelope expectations were corrected; their 24-case
  selection passed. The 106 worker cases passed after optimizing debug hashing.

Removed three obsolete cancellation tests that required aborting an active
browser call. Retained permit-lifetime and operation-ownership tests verify
that cancellation stops follow-up work while current calls keep their owners
until they settle. Pool fixtures now use automatic selection with distinct
priorities; pinned-account refusal is covered separately. Cooling assertions
check the HTTP response produced by the current completion boundary.

GitHub authentication uses `diddletime`, with the private build fork configured.
Both available accounts have Actions billing restrictions. The workflow exists,
but no hosted build has succeeded. The native CLI only validates configuration;
these fixtures do not establish live native delivery.

Native Python authority helpers were relocated without enabling them. They are
source-only; no native host launcher or monkeypatch installer is supplied.

The host now opens the SQLite journal and commits request charges and mint
reservations before external work. It preserves device selection, request
windows, cooldowns and reserve identities/counts. Reboot handling conservatively
rebases durations without crediting uncertain downtime. Interrupted records
require confirmed cleanup; existing profiles without history require migration.
The old privileged process-memory restoration modules are removed.

All 73 current Python runtime tests pass, including nine rate-policy and seven
journal cases. Three further host integration tests pass: durable close with
shared account references, cancelled mint ownership and refusal before external
work on storage failure. No live account state has been opened or changed.

Graceful daemon shutdown is now connected to startup and requires no feature
flag. It seals admission, drains HTTP and detached application work, then joins
host cleanup. Explicit host shutdown finishes accepted commands; lost stdin
cancels follow-up work while retaining outstanding browser calls. Failed
closures remain dirty in the journal. Attached browsers are not terminated.
Four new Python shutdown cases, five application cases and four host cases
pass using synthetic accounts, loopback fixtures and temporary state. The
27 lifecycle cases were rerun after adjusting teardown for intentionally
unacknowledged browser disconnections. One manual host test remains ignored.
The application installs its signal handlers before host startup and routes
startup failures through the same cleanup. Invalid listen addresses fail before
opening stores. A complete development archive at `6859509` passed verification,
staging, host exit, invalid-address refusal without state creation, and SIGTERM
and Ctrl-C shutdown with empty temporary state. This is not live validation.

Automatic recovery now binds each journal record to its service lifetime.
It requires a new invocation with verified process cleanup, or a reboot, and
retains charges, device identities and reserve limits. Unknown ownership or
surviving children refuses recovery. Ten new cases cover this boundary and
schema migration. One new fixture timestamp expectation was corrected; its
six-case selection passed while the other 84 results remained unchanged.
Two actual disposable service invocations also passed: a surviving child
prevented recovery after host loss, and recovery succeeded after service cleanup.
The packaged user-service template is not installed on the running service.
A complete development archive at `cceeee8` also passed verification, staging,
schema 2 creation, host shutdown, invalid-startup refusal and both shutdown
signals. The archive contains the service template. Its unchanged Rust binary
was reused from the prior verified build; no compiler rerun was needed.

The managed-service activation command now checks package/history compatibility
before stopping anything, switches the release pointer and rolls back on failed
startup. Interrupted updates restore the previous package first. Nine isolated
cases cover activation, refusal, rollback, update locking and readiness identity;
the existing five release cases remain passing. A Rust health-header test also
passes. These checks use temporary files and a fake service adapter, not the
running service. The command requires the one-time legacy migration first.

The complete `6d3aad2` development archive also passed package verification,
staging, protocol 1 and schema 2 checks, host shutdown, invalid-startup refusal,
and SIGTERM/SIGINT shutdown. Both daemon starts returned their actual PID in
the health response. These checks used empty temporary state and a private
network; no live activation occurred.

Removed the obsolete parallel native checker, its seven test modules and
private fixture, and the startup/routing runners for combined-mode binaries.
These relied on native dispatch or CLI flags that the browser app no longer
provides. Their results remain in the historical notes. Current runtime tests
and Rust library tests are unchanged; no removed suite counts as validation
of the new architecture.

Removed the remaining legacy host assembler, one-use restoration modules,
comparison supervisors and combined application/worker measurement fixtures.
Their private tests exercised the retired host-patching workflow. Current
browser runtime tests, native worker code and Rust library tests are unchanged.
Historical measurements and source links are retained; this cleanup makes no
new memory or native-delivery claim.

Native device grants, SDK proofs and their four existing authority tests now
live in the native library. `DeviceOwner` supplies admission and cleanup to
native requests and workers explicitly. The browser interface no longer has
native methods; its obsolete refusal test and unused enrollment fixture hooks
are removed. The current Rust run passes 138 core, 72 host, 302 native, 251
server and 33 browser CLI tests. One manual host integration test is ignored.
Both debug application targets rebuild successfully and print their expected
CLI help. No new package or live-service acceptance is claimed by these builds.
Python is unchanged. A concrete durable native owner and standalone startup
remain unfinished; this interface change does not enable live native calls.

Next: first migration of existing account history and the running service.
Current legacy account history and first-migration handling remain unresolved;
no production restart is part of this structural work.

See [the design](rearchitecture.md) and [packages](releases.md). Earlier measured
results remain in [history](history/development-status-before-reorganization-20261025.md).
