# Browserless Node development snapshot

The `browserless-node` branch contains the TypeScript restart and the rename to
Framerize Remote. This is unfinished development work, not a release.

## Checked locally

- The full workspace build and static checks pass.
- Node 22.23.4 and 24.13.0 each pass all 293 server tests and 32 core tests
  without skips. These include JSON account selection, session isolation,
  private credential references and startup cancellation. CLI cancellation now
  covers asynchronous provider creation, server construction and pending listen.
  Late-created providers close before exit; cleanup failures remain fatal.
- Both runtimes pass the installed Remote/server flow with streaming, a tool
  round trip, cancellation, saved profiles and isolated credentials.
- The installed packages expose their types without private workspace
  dependencies. Remote is 23,122 bytes packed and has no runtime dependencies.
- TLS checks include a hostile insecure-TLS environment and verify that a
  failed connection is closed before backend shutdown completes.

The Framer backend tests use synthetic loopback endpoints and an injected
minter. The installed flow checks account authentication and the generic HTTP
provider. Neither result proves live Framer compatibility. Whole-project
release qualification, hosted CI and new native artifacts remain open.

## Still unfinished

- Expose account selection and a real Castle minter through the server CLI.
- Finish the real encoder delivery and Castle/provider integration.
- Validate live browserless inference with the account owner's procedure.
- Qualify hosted packages and native artifacts before publishing a release.

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
