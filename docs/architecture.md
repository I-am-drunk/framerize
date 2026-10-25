# Architecture

The current reorganization is in progress. See [the design](rearchitecture.md)
for decisions, boundaries and the remaining migration work.

- `apps/framerize`: the normal browser application and command-line interface.
- `apps/framerize-native`: native development tools, with no live account startup.
- `crates/framer-native`: native account, authorization, request, transport and worker code.
- `crates/framer-api`: Framer endpoint policy, browser chains and shared payload helpers.
- `crates/framer-core`: domain types, account selection and ownership primitives.
- `crates/framer-server`: normal HTTP routes and shared account admission.
- `crates/framer-host`: browser process integration.
- `crates/framer-store`: state and session storage.
- `runtime/framerize_host`: Python browser host, with account, browser, resource
  and transport modules.
- `scripts/package.py` and `scripts/release.py`: build a complete archive and
  verify it into a new release directory.
- `web`: the application interface.

One Cargo workspace and lockfile cover both applications. The default target is
browser-based. Native depends on the shared libraries; those libraries do not
depend on native. Their browser routes do not select a native provider.

The browser archive carries its matching host and UI. The executable resolves
both from its own package, independently of the working directory. Development
overrides are explicit command-line options. Native developer archives remain
configuration-only. See [application packages](releases.md).

Concurrency-limit storage is selected by application startup. Library gates
are in memory unless supplied a state-file path. Fixtures never inherit the
normal service's learned limits through an implicit home-directory lookup.

The browser host exposes only browser commands. Ordinary account activity and
pending RPC ownership live in `accounts/`; native observation and comparison
adapters are absent from both the Rust bridge and the Python browser package.
Native Python authority sources live separately in `runtime/framerize_native`
and have no live launcher or installation hook.

The host opens one SQLite journal outside the release directory. Account rate
decisions live in `accounts/rates.py`: request charges and mint reservations
commit before external work. Device choices, cooldowns and reserve usage persist
across browser closure. A cancelled mint retains its own RPC and pacing lock
until the actual operation ends. Confirmed browser closure records clean state.

The legacy process-memory restoration code is removed from the runtime.
Existing profiles without a journal require migration. Interrupted records
require confirmed cleanup before reuse. First migration remains unfinished;
this code is undeployed.

The browser application always tracks its requests and background work.
SIGTERM or Ctrl-C stops new HTTP admission, lets accepted responses finish,
and joins detached application work before shutting down the host. The host
joins raw browser calls and records confirmed account closure before exiting.
The Rust bridge waits for that process exit. An unconfirmed close is an error,
not a successful drain. Attached user browsers are disconnected, never closed.
Owned children use separate process groups so foreground terminal interrupts
reach the daemon's shutdown coordinator first. Forced exits still require
recovery and are not equivalent to this graceful path.

The journal records the service lifetime beside each account snapshot. A new
host verifies its parent is the service's main process, delegation is disabled,
and the cgroup contains only that parent and itself. Dirty history can recover
after a different invocation of the same service, or a kernel reboot. Surviving
children, unknown ownership and a different service prevent automatic recovery.
Recovery preserves charges and device identities and applies a fresh cooldown.

Journal schema 2 adds per-record runtime ownership. Schema 1 upgrades without
inventing ownership for old records. Older readers reject schema 2, so package
rollback must select a compatible reader. No caller-provided clean flag or
missing lock file substitutes for process cleanup.

`scripts/release.py activate` updates an already migrated user service. It
checks package and history compatibility before stopping the service, switches
the package symlink atomically, and restores the previous package if startup
fails. Readiness checks match the health endpoint's PID to systemd's main PID
and the selected executable. One persistent update record supports recovery
after interruption; account data is outside that transaction and is never
rolled back. First migration from the legacy service remains separate.

Earlier investigation notes are preserved in
[architecture history](history/architecture-before-reorganization-20261025.md).
