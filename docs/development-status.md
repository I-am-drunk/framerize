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
- Current browser server: 250 tests pass. Host: 73 tests pass. Browser CLI: 32
  tests pass. Gate selection: 43 tests pass, including three new persistence tests.
- Python runtime: 80 fixture tests pass across lifecycle, imports, browser setup,
  packaging, rate persistence, request ownership and safe diagnostics. The fetch fixture
  runs the packaged JavaScript using an inert Node harness.
- The TypeScript/Vite UI build passes. Both local development archives pass
  checksum verification, staging and executable startup; the packaged browser
  host starts and exits on EOF using fresh temporary state without a browser.
- After the host cut, a fresh browser development archive passes the same checks
  and excludes observation/native packages. Debug builds remain explicitly
  marked as development even from a clean checkout; its regression test passes.
  Packaging omits empty directories left by removed runtime modules.
- Native: 298 accepted cases across the collected worker and request selections.
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
Four new Python shutdown cases, four application cases and four host cases
pass using synthetic accounts, loopback fixtures and temporary state. The
27 lifecycle cases were rerun after adjusting teardown for intentionally
unacknowledged browser disconnections. One manual host test remains ignored.

Next: automatic cleanup verification and first-migration handling.
Current account history and first-migration handling remain unresolved;
no production restart is part of this structural work.

See [the design](rearchitecture.md) and [packages](releases.md). Earlier measured
results remain in [history](history/development-status-before-reorganization-20261025.md).
