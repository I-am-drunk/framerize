# Application packages

A browser release contains `bin/framerize`, `web/`, `runtime/`, build information
and a file manifest. The executable selects its own host and UI after resolving
symlinks. Account data belongs outside the release directory. A loose host in
`~/.framerize` no longer overrides packaged code.

Check the actual installed files before a service restart:

```sh
python3 -B scripts/release.py verify ~/.local/share/framerize/current
```

This checks the executable and full manifest through the current symlink.
A directory name or build-info commit alone does not establish binary identity.
It exits nonzero for replaced binaries without starting the app or changing
account state. Operators can install a pinned copy of this script outside the
release and invoke `verify` from systemd `ExecStartPre`. Preserve that copy
when changing the current symlink. Do not repair an active release in place;
stage its original verified archive at a fresh path and switch after drainage.

Build from a committed checkout:

```sh
npm --prefix web ci
npm --prefix web run build
python3 scripts/cargo_with_identity.py build --locked --release -p framerize
python3 scripts/package.py framerize
```

The archive and its SHA256 checksum are written to `dist/`. The build-fork
workflow runs this sequence and uploads both files. Native packages are built
separately and include the experimental standalone service and ledger runtime.
The pinned Rust toolchain applies to local builds, CI and build metadata.
Both available GitHub accounts currently have billing restrictions on Actions;
the workflow has not yet produced a hosted artifact.

Check a browser archive before using it:

```sh
python3 -I -B scripts/smoke_browser_package.py dist/framerize-linux-*.tar.gz
```

Browser CI runs this same command before uploading the archive. It verifies
the files, starts the packaged service with empty temporary account state,
checks its health PID and packaged UI, and exercises profile exclusion,
both shutdown signals and later profile reuse. It opens no browser and sends
no inference request. The local check still needs the normal process, file
and loopback permissions; it does not install or activate a service.

Build the native package with Node 24.13.0 and the pinned Rust toolchain:

```sh
cargo build --locked --release -p framerize-native
node scripts/offline_worker/bundle.mjs --output target/native-worker
python3 scripts/package.py framerize-native
```

The worker builder downloads the exact hashed public SDK files and installs
locked npm dependencies with lifecycle scripts disabled. It includes Node and
its license. A new output directory is required; existing bundles are preserved.
Use `--assets DIRECTORY --offline --cache NPM_CACHE` when the exact assets and
npm tarballs are already cached. Package creation accepts `--worker-bundle PATH`
for a prepared bundle outside the default directory. No account data belongs
in this bundle. The package still uses the host's Python 3.12 and system C libraries.

The native CI job runs
`scripts/smoke_native_package.py` on the archive before uploading it. This uses
synthetic account history, the packaged worker configuration and a local endpoint to check
startup, profile exclusion, invalid-request refusal and signal shutdown.
The same check accepts `--worker-config CONFIG` to verify a real worker's files
during startup while retaining synthetic account state and invalid requests.
This does not execute that worker or test SDK acceptance.

For an already migrated profile, the staged native executable accepts:

```sh
bin/framerize-native serve --db ACCOUNT_DATABASE --profiles PROFILE_DIRECTORY \
  --accounts ACCOUNT_UUID \
  --listen 127.0.0.1:18789 --python /usr/bin/python3
```

It finds `runtime/native.py` and the worker beside its package. `--runtime-script`
and `--worker-config` are explicit development overrides. The profile must have existing sessions and
durable device history, and no other process may own it. Missing history is
refused, not replaced with new identities. This is ledger-backed V3 serving;
the startup smoke does not execute an SDK or establish live acceptance.

`/api/offline/stream` also accepts an optional `continuation` object with
`max_output_tokens`, `prompt` and `contexts`. Supply one complete authorization
context for each further chunk, up to 15. The first context stays in the main
request envelope. The total budget is at most 32,768 tokens and the original
60-second deadline covers the whole chain. Only text cut off at its output
limit can continue; refusals and incomplete tool calls do not trigger another
attempt. `/api/offline/complete` rejects this option. This interface has only
synthetic validation so far.

The exceptional first migration can be selected with
`--expire-legacy-from PREVIOUS_RUNTIME_JSON`. That file must contain the actual
previous service's `boot_id`, `unit`, `cgroup` and `invocation`. The successor
must run in that same service with the old processes gone. The command does
not stop them, copy their sessions or infer an empty history.

Before opening its listener, native startup holds the session directory and
waits out the legacy 600-second rate window. Unknown daily reserve usage stays
unavailable for 24 hours. Cancellation joins the migration helper and publishes
no partial journal. A complete journal skips this first-migration path on later
starts. Where the prior primary device is unknown, the first native admission
persists a new choice from the preserved ready-device inventory. A known but
unavailable primary still refuses work. This option has not migrated production
state and is not permission to interrupt the working browser service.

Stage a downloaded package with Python 3.12 or later:

```sh
python3 scripts/release.py stage ARCHIVE --sha256 CHECKSUM
```

Staging verifies the archive and every packaged file, then installs a new release
directory. It needs no sudo, does not overwrite an existing release and does not
change the running service. `scripts/linux/server-setup.sh` delegates to it.

After the one-time service and history migration, activate a staged production
browser package with:

```sh
python3 scripts/release.py activate ~/.local/share/framerize/releases/RELEASE
```

The command checks both packages, the installed user unit and the live history
schema before stopping anything. It drains the service, changes `current`
atomically, starts the selected package and verifies that the health response
comes from its main process. Failed startup restores the previous compatible
package. Account history is never restored from a backup or changed by this tool.

One update lock prevents concurrent replacements. An interrupted update leaves
`update.json`; rerunning the command restores the previous package first and
reports `recovered`, without retrying the failed candidate. Both packages must
declare activation protocol 1, use the same history schema and carry the same
installed service template. Development packages and unreviewed unit overrides
are refused. This command does not perform the unresolved legacy migration.

For source development, pass `--web-dir web/dist` and
`--sidecar-script "$PWD/runtime/host.py"` explicitly. `--allow-dirty` creates a
package marked for local development.
Debug builds also require that flag and remain marked as development packages
even from a clean checkout. Build information records the profile independently
of whether the source tree was dirty.

The application keeps its learned concurrency limits in
`~/.framerize/gate-state.json`, or the explicit `FRAMERIZE_GATE_STATE` path.
It loads this state once at startup and replaces the file atomically after
overload evidence. Library consumers and tests use in-memory gates by default.
This file contains concurrency estimates, not the account's request history.

The current service has not undergone its first migration. The new
durable rate ledger, graceful drain and service-bound crash recovery are
implemented. Legacy history migration must precede promotion. Staging is not
deployment.

Send SIGTERM to the daemon to request graceful shutdown. It stops admitting
requests, waits for accepted work, and then closes owned browsers and the host.
Foreground Ctrl-C follows the same path. Attached user browsers remain open.
An error during cleanup is reported as an unsuccessful shutdown. Service-manager
timeouts and forced termination are not a substitute for confirmed cleanup.

Browser archives include `share/systemd/user/framerize.service`. The template
uses the selected `~/.local/share/framerize/current` package and the existing
`~/.framerize` data paths. It sends graceful termination to the daemon, allows
five minutes for draining, then removes surviving processes. It requires no
sudo. Activation requires this managed unit to be installed by the first
migration; the updater does not overwrite an unfamiliar unit.

Build information declares `rate_history_schema: 2`. This schema records each
account's runtime owner alongside its history. Readers from before this change
cannot reopen it. Keep account state in place and roll back only to a compatible
package. Upgrading schema 1 adds ownership metadata without inventing ownership
for old rows. Existing profiles without any journal still require migration.
