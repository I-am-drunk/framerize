# TypeScript project layout

The restart keeps one repository with separate runtime packages. Private
fixture assets stay in place while runtime code migrates incrementally.

| Directory | Responsibility |
| --- | --- |
| `framerize-core/` | Shared HTTP instance, contracts, and protocol rules. No accounts or provider implementation. |
| `framerize-server/` | Browserless Node server, account ownership, and provider requests. |
| `framerize-castle/` | Castle Library, used by the server rather than the laptop package. |
| `framerize-remote/` | Local Framerize instance that forwards inference to the remote server. |

Remote is not merely a setup command. It exposes the local client-facing API.
The server performs inference; Remote carries requests and streamed responses.
They must not grow separate definitions of the same public behavior.

## First migration step

The private root npm workspace includes `framerize-core` and `framerize-remote`.
The server retains its package and lockfile, but its HTTP factory now uses
the bundled TypeScript core. Root `build`, `check`, and `test` cover core,
server, Remote, Castle Library and packaging tools. They do not prove live provider compatibility.

TypeScript, Node type declarations, and esbuild are development-only
dependencies. The shared package emits ESM and `.d.ts` files under `dist/`.
Tests compile to an ignored directory excluded from package contents.
Root packaging tools also compile to the ignored `.tools/` directory.

After building, `npm run check:package` packs the core, installs it offline
outside the workspace, compares installed files with compiled inputs, and
runs authenticated HTTP and shutdown checks. It uses private npm files and
cleans its temporary directory on success or failure. Runtime dependencies
are forbidden; archive limits are 64 KiB packed and 256 KiB unpacked.

Before installation, the checker copies source and build configuration into
a separate directory, rebuilds with the pinned TypeScript compiler, and packs
that clean output. Both archives must match byte for byte. This catches stale
declarations and removed modules left in `dist/`. The report includes the clean
build's archive hash and compiler version. It does not certify a different
compiler, operating system, or application package without running that check.

The `typescript-core` workflow runs build, type checks, tests, and this
installed-package check on Linux, macOS, and Windows with Node 22.23.4 and
24.13.0. Each successful job retains a report with source identity, archive
hash and size, runtime version, and smoke results. The workflow does not
build native binaries, publish packages, or qualify the complete application.

The shared backend contract matches the accepted `forward`, `ready`, and
`close` lifecycle. It passes request cancellation and opaque Buffer streams.
It deliberately contains no credential loader, account database, HTTP client,
browser process, Castle asset, or global runtime singleton.

## Integration still required

The server now calls the shared `createInstance` listener. Its former HTTP
transport and lifecycle modules are removed. Configuration validation,
server-token loading, JSON account metadata, upstream HTTP provider, shared
errors, CLI, logger and server composition now compile from strict TypeScript.
The small Node launcher and existing build/test tools remain JavaScript.
All package runtime exports resolve to generated modules under `dist/`.
Remote now uses the same listener with a TypeScript
forwarding backend. It authenticates locally and substitutes the remote token.
An installed-server fixture now starts the real CLI and real HTTP provider
against a loopback synthetic upstream. It verifies a streamed tool call and
its follow-up result, unchanged request/response bytes, separate credentials,
local authentication refusal, unavailable readiness, and disconnect propagation.
The test closes the process and checks that its port and network handles are
released. POSIX runs require graceful exit zero; Windows checks termination
without claiming graceful SIGTERM handling. Root tests and core CI include it.
The installed-pair test reuses this control through the actual Remote CLI. It
checks readiness before inference and rejects unavailable peers without a
provider request. It does not establish live browserless compatibility.
The listener has no environment loading, account access, provider HTTP client,
or process-level signal handlers. Its caller owns configuration and process
startup; the instance owns request cancellation and backend shutdown.

Loopback tests compare 28 captured responses from the accepted JavaScript
server and forward through the migrated server with provider/probe adapters.
They check opaque streamed bytes, independent credentials, readiness, and
cancellation. The current remote tests use the production Remote backend.
Installed-pair tests also compile a standalone TypeScript consumer without
the private core dependency and check hostile environment proxy isolation.

The root build bundles only core's listener into `framerize-server/dist/core.js`.
The npm file list includes that bundle, compiled server modules and their
declarations explicitly, not the entire directory of native-build artifacts.
The build copies the required core declarations into `dist/core-types/` and
writes `dist/core.d.ts` before compiling the provider. Installed consumers need
Node types for TypeScript checking but no private workspace package at runtime
or compile time. The provider keeps optional headers and cancellation signals
for direct callers and implements the stricter shared backend contract.
TypeScript source is excluded. Installed servers have no private workspace dependency.
CI rebuilds this file before packaging; downloaded-package checks compare its
bytes with the current build and compare ordinary source files with Git.

Remote must not send requests directly to inference providers, read server
accounts, or include Castle in its dependency graph. Its remote credentials
must remain separate from the credentials that protect its local listener.

The listener authenticates inbound clients. A remote backend receives only
allowlisted request headers and supplies its own remote Bearer token. It must
not reconstruct or forward the caller's Authorization header.

Remote transports retain complete response headers, including Connection,
under a finite parser-byte cap until the listener applies its allowlist.
Disabling header-count truncation does not remove the byte cap. The remote
backend must not discard Connection before filtering its nominated fields.

Remote forwards response bytes without parsing SSE or inventing terminal
events. It does not need the old SSE parser in the shared core. CLI logs use
fixed diagnostics and core's allowlisted events, never raw request data or
caught exception messages.

Readiness is a finite control request with byte and total-time limits.
Inference streaming must retain cancellation, backpressure, and idle limits
without imposing a total deadline on a healthy stream.

Bundle private shared code into released consumers so npm installs do not
depend on workspace symlinks. Validate the actual tarball outside this repo.
Retain the browserless server's standalone executable and a reproducible
Framer Next recovery build, rather than another independently maintained API.

Root commands passing covers the stated synthetic scenarios only. It does not
establish browserless provider integration, broad local/remote parity, live
account behavior, npm publication, or deployment readiness. Legacy Remote
native-release and package-coexistence automation still needs migration to
the new CLI before this restart can be published.
