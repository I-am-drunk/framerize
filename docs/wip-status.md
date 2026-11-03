# Framer Next development snapshot

The `framer-next` branch contains the TypeScript restart and the rename to
Framerize Remote. This is unfinished development work, not a release.

## Checked locally

- The full workspace build and static checks pass.
- Node 22.23.4 and 24.13.0 each pass all 287 server tests and 32 core tests
  without skips. These include JSON account selection, session isolation,
  private credential references and startup cancellation.
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
The Windows installed-pair failure lost its redirected output. That check now
copies output into the job log while preserving a failed exit status; its
cause remains unverified until the next hosted run.
