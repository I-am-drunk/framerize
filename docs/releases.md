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
workflow runs this sequence and uploads both files. Native developer packages
are built separately; their CLI currently checks configuration structure only.
The pinned Rust toolchain applies to local builds, CI and build metadata.
Both available GitHub accounts currently have billing restrictions on Actions;
the workflow has not yet produced a hosted artifact.

Stage a downloaded package with Python 3.12 or later:

```sh
python3 scripts/release.py stage ARCHIVE --sha256 CHECKSUM
```

Staging verifies the archive and every packaged file, then installs a new release
directory. It needs no sudo, does not overwrite an existing release and does not
change the running service. `scripts/linux/server-setup.sh` delegates to it.

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

Activation and migration of the current service are still unfinished. The new
durable rate ledger and graceful drain are implemented, but migration and
crash recovery must be completed before promotion. Staging is not deployment.

Send SIGTERM to the daemon to request graceful shutdown. It stops admitting
requests, waits for accepted work, and then closes owned browsers and the host.
Foreground Ctrl-C follows the same path. Attached user browsers remain open.
An error during cleanup is reported as an unsuccessful shutdown. Service-manager
timeouts and forced termination are not a substitute for confirmed cleanup.
