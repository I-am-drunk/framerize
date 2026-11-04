# TypeScript project layout

The repository contains a shared HTTP core and separate server and Remote
packages. Their runtime code compiles from TypeScript. Small JavaScript
launchers remain in each application's `bin/` directory.

| Directory | Responsibility |
| --- | --- |
| `framerize-core/` | Shared HTTP listener, client authentication, request limits and backend contracts. |
| `framerize-server/` | Provider accounts, session authentication, Castle and upstream inference requests. |
| `framerize-remote/` | Local client API that forwards requests to the server. |
| `scripts/` | Build, package and verification tools, not application runtime dependencies. |

The server and Remote use the same HTTP listener. Their backends implement
`forward`, `ready` and `close`, including cancellation and streaming cleanup.
The application owns configuration and process signal handlers.

Remote does not load provider accounts or Castle. It authenticates local
clients and uses a separate credential for the server. Neither backend
receives the client's authorization header from the shared listener.

## Builds

The private root npm workspace includes `framerize-core` and `framerize-remote`.
The server has its own package and lockfile. Use Node.js 22.12 or newer and
run these commands from the repository root:

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm run build
```

Root `build` compiles the packages and verification tools. Use
`npm run build:server` for the server and its build tools without building
Remote.

TypeScript, Node type declarations, and esbuild are development-only
dependencies. The shared package emits ESM and `.d.ts` files under `dist/`.

Castle Library is included in `framerize-server/src/castle.ts`, including its
collection and encoding workers. Regenerate it from the pinned commit in an
existing local Castle checkout:

```sh
node scripts/vendor-castle-library.mjs /path/to/castle
node scripts/vendor-castle-library.mjs /path/to/castle --check
```

The generator bundles source and assets from `typescript-castle/`, records their
hashes in `castle-library-source.json`, and ports the original behavior tests.
Normal builds do not fetch or read another repository. Tests check fixed encoder
outputs, validation, cancellation, worker retirement, metadata and standalone
use. Hashes detect byte changes, not complete behavioral equivalence or live
provider acceptance. Embedded asset hashes detect corruption, not tampering by
someone who can also edit the bundled manifest.
Tests compile to an ignored directory excluded from package contents.
Root packaging tools also compile to the ignored `.tools/` directory.

## Installed packages

The server build bundles core's listener as `framerize-server/dist/core.js`
and copies the required declarations into `dist/core-types/`. Server exports
resolve to compiled modules under `dist/`; its npm archive excludes TypeScript
source and includes the Castle runtime as `dist/castle.js`.

Remote bundles the shared core into its own runtime and copies core declarations
into its package. Neither installed application needs a private workspace
symlink or a separately installed core package. Their package manifests define
the shipped files; build tools are not runtime dependencies.

## Verification

After building, `npm run check:package` verifies the core package outside the
workspace, including its clean rebuild, archive bytes and HTTP lifecycle.
The [package checks](package-coexistence.md) cover installed server and Remote
use. The [native build guide](native-server-build.md) covers standalone server
artifacts. Use the [server guide](../framerize-server/README.md#verify-locally)
for package-local checks.

Source tests, installed packages, native executables and live provider requests
are separate forms of evidence. A passing result applies only to its tested
revision, artifacts, runtime and platform. The
[development status](wip-status.md) lists the remaining release requirements.
