# Browserless Node development snapshot

The `main` branch contains the TypeScript restart and the rename to
Framerize Remote. This is unfinished development work, not a release.

## Earlier local checks

- The server build and workspace static checks pass.
- Node 22.23.4 and 24.13.0 each pass 357 server, workflow and installed-package
  checks without skips. They cover JSON account selection, session isolation,
  private credential references and startup cancellation. CLI cancellation now
  covers asynchronous provider creation, server construction and pending listen.
  Late-created providers close before exit; cleanup failures remain fatal.
- Both runtimes pass the installed Remote/server flow with streaming, a tool
  round trip, cancellation, saved profiles and isolated credentials.
- The installed packages expose their types without private workspace
  dependencies. Remote is 23,122 bytes packed and has no runtime dependencies.
- TLS checks include a hostile insecure-TLS environment and verify that a
  failed connection is closed before backend shutdown completes.

The installed account flow uses synthetic loopback endpoints and the built-in
Castle runtime. Castle is now compiled directly into the server. Earlier native
results covered a separately extracted runtime and do not qualify this changed
implementation. Validate each new native artifact before release.

## Still unfinished

- Validate live browserless inference with the account owner's procedure.
- Qualify hosted packages and native artifacts before publishing a release.
- Publish the qualified Remote package to npm.

The [single-request validator](inference-validation.md) is ready for an approved
endpoint and account. Its 23 offline checks pass on Node 22.23.4 and 24.13.0.
No live inference was performed for that validation checkpoint.

## Earlier portability fixes

The first hosted snapshot exposed a Windows type-fixture path mismatch and
release fixtures inheriting the enclosing CI commit. Both are fixed locally.
Retained follow-up logs identified CRLF-sensitive license and workflow checks,
a Windows preload path that needed a file URL, a macOS symlink-path expectation,
and a Windows resource assertion that ran before the closed handle retired.
The corrected checks keep exact license bytes and require all resources to
close within one second. A live-listener negative control still fails.

Node 22.23.4 and 24.13.0 each pass 76 workflow-policy checks, 12 first-run checks,
and six installed-package and cleanup checks. The installed flow also covers
paths containing spaces, `#` and `%`. Product runtime source is unchanged.
Hosted confirmation of these portability corrections remains pending.
