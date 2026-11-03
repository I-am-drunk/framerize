# Offline library fixture CI

The private `framerize-castle` package has a separate `library-ci` workflow.
It runs the existing five fixtures on Ubuntu 22.04, macOS 15 and Windows 2022,
with Node 22.12.0 and Node 24. No package installation, native build, secrets,
publishing, deployment or live endpoint is involved. Checkout and setup-node
are pinned, and checkout does not persist credentials.

The workflow runs on relevant pull requests, pushes to `main` and `framer-next`,
and manual dispatch. Its path filters include the library, workflow, policy
test and this document. Permissions are limited to `contents: read`.

## Local commands

Use a supported Node executable. Clear inherited `NODE_OPTIONS` before starting
Node, since a preload can execute before any JavaScript guard.

```sh
unset NODE_OPTIONS
node --test test/library-workflow.test.mjs
cd framerize-castle
node test/helpers/offline-guard.mjs
```

The helper launches `node --test test/castle.test.js` from the package directory.
It creates a private temporary directory, replaces the child environment with
an allowlist, and sets HOME, configuration and temporary paths there. It sets
`FRAMERIZE_REALM_STATE` to a fresh file path before imports, not the repository
default. The directory is removed after the subprocess returns, including on
test failure. No operator credentials, proxy settings or seed state survive.

## Containment

An inherited `NODE_OPTIONS` preload denies Node TCP, TLS, HTTP, HTTP/2, UDP,
DNS, fetch and WebSocket operations before they connect or listen.
Filesystem read/open entry points allow only package files and private fixture
files. They explicitly deny `/tmp/font-probe-capture.json` and the repository
default realm-state path without reading either. The optional capture remains
absent to the existing library rather than supplying fabricated capture data.

Subprocess entry points retain the preload and private environment even when
callers supply their own environment. Only the current Node executable and
private scripts with its exact shebang are allowed. Shell commands are denied.
The encoder fixture still runs as a real process; no library result is mocked.

This is an accidental-I/O guard for reviewed, builtin-only fixtures, not an OS
sandbox for hostile JavaScript, native addons or arbitrary executables. Keep
new fixture dependencies and I/O entry points under review. The policy tests
use inert denied operations, including a read sentinel that proves the real
file-read function is never reached for the unmanaged capture.
Network regression arguments are deliberately invalid, so a missing guard
fails without making a connection. Filesystem integration probes use fresh,
nonexistent sibling paths. The exact unmanaged path is tested with an inert
read callback, never by probing the real file.

## Windows limits

The existing tests deliberately skip three POSIX shebang cases on Windows:
encoder success, concurrent mint rejection, and encoder timeout termination.
Windows runs the required-encoder and missing-encoder cases. A green Windows
job does not qualify encoder success, concurrency or timeout termination.
The guard's inert subprocess tests use Node directly on Windows and do not
replace or remove those three skips.

## Evidence

Local Linux validation on 2026-11-03 passed with Node 22.23.4 and 24.13.0:
25 policy/guard tests and all five existing library fixtures per version,
with zero skips or failures. Both fixture launches reported complete cleanup;
no private fixture directories remained. Syntax and diff checks also passed.
Hosted platform results are not inferred from these local runs.

Local and hosted results are separate gates. Local commands must report the
exact Node versions, fixture counts and skips; hosted jobs supply OS evidence.
Passing these fixtures does not demonstrate live delivery, remote readiness,
anti-bot acceptance or production activation. Existing runtime and fixture
source files remain unchanged.
