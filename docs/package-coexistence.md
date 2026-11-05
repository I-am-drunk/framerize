# Check the two installed packages

Run this checker against two existing, reviewed npm tarballs. It does not pack
new tarballs, publish packages, start services, or contact inference endpoints.

```sh
node node_modules/typescript/bin/tsc -p tsconfig.tools.json
node .tools/scripts/package-coexistence.js /path/to/framerize-remote-0.2.0.tgz /path/to/framerize-0.1.0.tgz
```

Both arguments are required. Relative paths resolve from the caller's current
directory; absolute paths and paths containing spaces also work. The order does
not matter. Use Node 22.12 or newer, with npm installed in that Node distribution,
with the compiled tooling and its ESM package marker. The checker reads bounded
USTAR archives directly. It rejects links, special files and unsafe paths before installation.

## What it checks

1. Snapshot and hash `framerize` and `framerize-remote`. Require both `frm` and
   `framerize` aliases and no runtime, optional, or peer dependencies.
2. Install the libraries together with command links disabled. Install each CLI
   in its own global prefix, offline, with lifecycle scripts disabled and empty
   caches and configuration.
3. Compare every installed file to its tarball bytes. The only allowed change is
   npm's removal of one CR before the first newline of a launcher CRLF shebang.
   All other source bytes, including body line endings, must remain unchanged.
4. Verify each prefix's command targets, direct help, offline `npm exec` and `npx`,
   the Remote version and setup help, and current module exports. Remote readiness
   and serving run in the separate bounded loopback fixture, not this no-network check.
5. Report package name, version, input filename and SHA-256, file count, Node/npm
   versions, and host platform. Delete the temporary install after either result.

The server probe covers `.`, `./config`, `./accounts`, `./logger`, and `./provider`.
Castle is internal to the server package and has no public package export.
The package checker and transport tests do not exercise Castle minting.
Passing them does not qualify the Castle runtime or live-provider compatibility.

Subprocesses receive a fresh environment and temporary home, not inherited
account paths, credentials, proxy settings, npm configuration, or Node options.
A Node preload rejects sockets, listeners, HTTP requests, UDP traffic and fetch.
This protects fixture execution from accidental network use. It is not an OS
sandbox for malicious packages; use reviewed artifacts only.

## Focused regression check

```sh
npm run test:packages
```

The separate [package-pair CI workflow](package-coexistence-ci.md) has
[retained three-OS qualification](package-coexistence-evidence/41667971716/README.md)
for the recorded baseline pair. This checker does not change either
package's runtime or qualify native archives. Retain its JSON
output with externally verified source provenance; a tarball filename alone
does not establish a Git commit. Success is not publication or activation approval.

## Changing the package contract

The current checker requires dependency-free packages, the existing executable
names, and the exports listed above. Only the server runs its configuration-only
`check` command here. Remote exposes `createRemoteBackend` and `parseRemoteEndpoint`.
Its backend is constructed and closed without making a request.

Update the checker and its tests together when either public interface changes.
Do not weaken credential validation or enable network access to pass this gate.
The byte check discovers all archive members, so added bundled files are not
limited to the 13 files in historical evidence. New package bytes still require
fresh qualification; the retained reports do not qualify later runtime changes.

## Historical reports

The [three-platform report](package-coexistence-evidence/41667971716/README.md)
and [later Linux report](package-coexistence-evidence/2026-11-03-final-pair/README.md)
retain source identities, artifact hashes and results for their recorded
package pairs. They do not qualify the current packages, native executables
or a release.

The earlier development timeline remains in Git history. Use the current
[development status](wip-status.md) for release prerequisites rather than old
package names or historical test counts.
