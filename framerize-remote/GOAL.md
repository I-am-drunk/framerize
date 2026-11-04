# Framerize Remote objective

Maintain Framerize Remote as a local Framerize instance that runs the shared
`@framerize/core` listener and forwards inference to one configured remote
Framerize server.

## Ownership boundary

Keep the package in `framerize-remote/` with its TypeScript source, tests,
build script, and documentation. Do not implement accounts, sessions,
minting, inference, Castle runtime, or provider integration here. Those belong
in the server. Core owns only the shared HTTP contract and listener.
Remote adds a forwarding backend and a thin CLI.

## Dependencies

Depend on `@framerize/core` at build time and bundle the used core code into
the released output. The installed package must have no runtime dependency on
a workspace path and no dependency-heavy CLI framework. Keep zero runtime
dependencies.

## Verification

Build compiled output and declarations. Test on Node 22 and 24 with synthetic
secrets, private temporary files, and loopback servers only. Prove streamed
byte preservation, local/remote credential separation, readiness failures with
zero inference, cancellation, and process cleanup. Pack and install the package
outside the workspace with scripts disabled, then run the real
help/version/check/serve path without a compiler or workspace link.

Do not make live external inference requests, build native executables,
activate a service, or publish a release without owner approval.
