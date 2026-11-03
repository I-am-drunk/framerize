# Framer Next development snapshot

The `framer-next` branch contains the TypeScript restart and the rename to
Framerize Remote. This is unfinished development work, not a release.

## Checked locally

- The full workspace build and static checks pass.
- Node 22.23.4 and 24.13.0 each pass 132 focused server runtime tests without
  skips, including the Framer request mapping and session authentication.
- Both runtimes pass the installed Remote/server flow with streaming, a tool
  round trip, cancellation, saved profiles and isolated credentials.
- The installed packages expose their types without private workspace
  dependencies. Remote is 23,122 bytes packed and has no runtime dependencies.
- TLS checks include a hostile insecure-TLS environment and verify that a
  failed connection is closed before backend shutdown completes.

The Framer backend tests use synthetic loopback endpoints and an injected
minter. The installed flow uses the generic HTTP provider. Neither result
proves live Framer compatibility. The complete repository suite, hosted CI
and new native artifacts have not been qualified for this snapshot.

## Still unfinished

- Connect private account configuration to the Framer backend and server CLI.
- Finish the real encoder delivery and Castle/provider integration.
- Validate live browserless inference with the account owner's procedure.
- Qualify hosted packages and native artifacts before publishing a release.
