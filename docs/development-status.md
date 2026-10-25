# Development status

The reorganization is on `refactor/separate-apps-and-ci-20261025`, in draft
[PR #2](https://github.com/I-am-drunk/framerize/pull/2). It has not been deployed.

Completed source changes:
- Separate browser and native applications in one workspace. The browser
  executable does not depend on the native implementation.
- Native code grouped by account, authorization, request, transport and worker.
- A Python runtime package with account, browser, resources, wire and legacy
  restoration modules. Runtime imports do not launch browsers.
- Complete executable, host and UI archives, checksum manifests, and a staging
  command that leaves the running service untouched.
- Explicit host/UI development overrides. Installed applications select the
  assets next to their canonical executable.
- Explicit concurrency-limit persistence at application startup, atomic file
  replacement, and restoration of the actual permit count.

Validation collected locally:
- The workspace all-target check passed at the application split milestone.
- Current browser server: 250 tests pass. Host: 151 tests pass. Browser CLI: 28
  tests pass. Gate selection: 43 tests pass, including three new persistence tests.
- Python runtime: 40 fixture tests pass across lifecycle, imports, browser setup
  and package staging. Reused unchanged results are retained.
- The TypeScript/Vite UI build passes. Both local development archives pass
  checksum verification, staging and executable startup; the packaged browser
  host starts and exits on EOF using fresh temporary state without a browser.
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

Next: remove remaining experimental adapters from the normal host, and
implement durable account rate history plus graceful
shutdown. Current account history and first-migration handling remain unresolved;
no production restart is part of this structural work.

See [the design](rearchitecture.md) and [packages](releases.md). Earlier measured
results remain in [history](history/development-status-before-reorganization-20261025.md).
