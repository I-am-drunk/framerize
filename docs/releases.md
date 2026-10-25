# Application packages

A browser release contains `bin/framerize`, `web/`, `runtime/`, build information
and a file manifest. The executable selects its own host and UI after resolving
symlinks. Account data belongs outside the release directory. A loose host in
`~/.framerize` no longer overrides packaged code.

Build from a committed checkout:

```sh
npm --prefix web ci
npm --prefix web run build
cargo build --locked --release -p framerize
python3 scripts/package.py framerize
```

The archive and its SHA256 checksum are written to `dist/`. The build-fork
workflow runs this sequence and uploads both files. Native packages are built
separately and include the experimental standalone service and ledger runtime.
The pinned Rust toolchain applies to local builds, CI and build metadata.
Both available GitHub accounts currently have billing restrictions on Actions;
the workflow has not yet produced a hosted artifact.

Build the native package with `cargo build --locked --release -p framerize-native`
and `python3 scripts/package.py framerize-native`. The native CI job runs
`scripts/smoke_native_package.py` on the archive before uploading it. This uses
synthetic account history, an inert worker and a local endpoint to check
startup, profile exclusion, invalid-request refusal and signal shutdown.

For an already migrated profile, the staged native executable accepts:

```sh
bin/framerize-native serve --db ACCOUNT_DATABASE --profiles PROFILE_DIRECTORY \
  --worker-config VERIFIED_WORKER_CONFIG --accounts ACCOUNT_UUID \
  --listen 127.0.0.1:18789 --python /usr/bin/python3
```

It finds `runtime/native.py` beside its package. `--runtime-script` is an
explicit development override. The profile must have existing sessions and
durable device history, and no other process may own it. Missing history is
refused, not replaced with new identities. This is ledger-backed V3 serving;
the startup smoke does not execute an SDK or establish live acceptance.

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
