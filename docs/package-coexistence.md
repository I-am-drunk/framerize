# Check the two installed packages

Run this checker against two existing, reviewed npm tarballs. It does not pack
new tarballs, publish packages, start services, or contact inference endpoints.

```sh
node scripts/package-coexistence.mjs /path/to/framerize-nexus-0.1.9.tgz /path/to/framerize-server-0.1.0.tgz
```

Both arguments are required. Relative paths resolve from the caller's current
directory; absolute paths and paths containing spaces also work. The order does
not matter. Use Node 22.12 or newer, with npm installed in that Node distribution,
and `tar` on PATH. No root package.json or npm workspace is needed.

## What it checks

1. Snapshot and hash the exact inputs, and require one package of each name with
   distinct executable names and no runtime, optional, or peer dependencies.
2. Install both snapshots together into a fresh temporary directory using npm's
   offline mode, disabled lifecycle scripts, and an empty cache and configuration.
3. Compare every installed file to its tarball bytes. The only allowed change is
   npm's removal of one CR before the first newline of a launcher CRLF shebang.
   All other source bytes, including body line endings, must remain unchanged.
4. Verify distinct executable targets, direct CLI help, offline `npm exec` help,
   both CLI `check` commands, and the two Nexus and five baseline server module
   exports. The probe only imports providers. CLI checks initialize and close
   configuration without making inference requests. Default ports remain 8788 and 8789.
5. Report package name, version, input filename and SHA-256, file count, Node/npm
   versions, and host platform. Delete the temporary install after either result.

The server probe covers `.`, `./config`, `./accounts`, `./logger`, and `./provider`.
It does not probe the later `./castle` or `./package.json` exports. Extending
that list requires the parent-owned package contract decision; the existing
results do not qualify the added library or its integration.

Subprocesses receive a fresh environment and temporary home, not inherited
account paths, credentials, proxy settings, npm configuration, or Node options.
A Node preload rejects sockets, listeners, HTTP requests, UDP traffic and fetch.
This protects fixture execution from accidental network use. It is not an OS
sandbox for malicious packages; use reviewed artifacts only.

## Focused regression check

```sh
node --test test/package-coexistence.test.mjs
```

The parent has authorized a separate [package-pair CI workflow](package-coexistence-ci.md),
whose three-OS qualification remains pending. This checker does not change either
package's existing workflow or qualify native archives. Retain its JSON
output with externally verified source provenance; a tarball filename alone
does not establish a Git commit. Success is not publication or activation approval.

## Changing the package contract

The current checker requires dependency-free packages, the existing executable
names, and the baseline exports listed above. It invokes each CLI with
`check --host 127.0.0.1 --upstream http://127.0.0.1:1` in a private environment
without credential references. It also calls Nexus `parseConfig()` and server
`loadConfig()` without arguments and checks default ports 8788 and 8789.

These are current fixture assumptions, not constraints on the parent-owned
thin-Nexus design. If that design changes these interfaces or requires fixture
configuration, update the checker and its tests in a separately reviewed change.
Do not weaken credential validation or enable network access to pass this gate.
The byte check discovers all archive members, so added bundled files are not
limited to the 13 files in historical evidence. New package bytes still require
fresh qualification; the retained reports do not qualify later runtime changes.

## Recorded Linux verification

The [final retained package-pair check](package-coexistence-evidence/2026-11-03-final-pair/README.md)
records the later server `23d87c6c` artifact with the same Nexus artifact on
Linux Node 22 and 24. The earlier package-pair evidence below remains historical.

On 2026-11-03, all 10 focused tests passed on both Node 22.23.4 with npm 10.9.9
and Node 24.13.0 with npm 11.6.2. Neither run skipped tests. This includes real
offline fixture installs and the launcher-only CRLF allowance under both npm
major versions. The runtime and checker source were unchanged between runs.

On Node 22.23.4, the checker also passed against these existing tarballs,
without repacking:

- `framerize-nexus@0.1.9`, source `671ed7df4507b391fc6efc72bf919aebd289a1eb`.
  SHA-256 `12c6eb43755f16a858d12b150bfde44622298ab2fcf489d03abadba0bd4fe528`.
- `framerize-server@0.1.0`, source `0848e05a0daf5b55f089fda3aeb7d79b7c2259c9`.
  SHA-256 `7857503139d2f48dddfe0adb8967f1b19dd2d853d4641f9543642a160669d5da`.

A separate raw Git-blob comparison matched all 13 archived files per package
to those exact commits. Both installed packages passed with deliberately poisoned
parent account, endpoint, host, port and credential environment variables.
The fixtures also cover LF/CRLF bytes, disabled lifecycle scripts, invalid names
and dependencies, missing public exports, and rejected socket/listener/fetch calls.

This is Linux x64 npm-install evidence only. It does not qualify this checker on
macOS or Windows, or any later source tree. Native and aggregate release evidence
belongs to the separate packaging checks.

The regression fixture passes an archive basename to `tar` and sets its working
directory separately. This avoids GNU tar treating a Windows drive-letter
prefix as a remote archive address when tests run under Git Bash. All 10 focused
tests pass with this invocation on Linux using Node 22.23.4. The checker itself
is unchanged. This fixture correction does not establish Windows or
macOS qualification; those platforms still need their own test evidence.
