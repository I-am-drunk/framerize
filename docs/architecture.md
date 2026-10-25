# Architecture

The current reorganization is in progress. See [the design](rearchitecture.md)
for decisions, boundaries and the remaining migration work.

- `apps/framerize`: the normal browser application and command-line interface.
- `apps/framerize-native`: the experimental standalone native service and tools.
- `crates/framer-native`: native account, authorization, request, transport and worker code.
- `crates/framer-api`: Framer endpoint policy, browser chains and shared payload helpers.
- `crates/framer-core`: domain types, account selection and ownership primitives.
- `crates/framer-server`: normal HTTP routes and browser account admission.
- `crates/framer-host`: browser process integration.
- `crates/framer-store`: state and session storage.
- `runtime/framerize_host`: Python browser host, with account, browser, resource
  and transport modules.
- `runtime/framerize_accounts`: shared journal, rate policy and crash recovery.
- `runtime/framerize_native`: native ledger ownership, independent of the browser host.
- `runtime/framerize_runtime`: process framing and package verification shared by both apps.
- `scripts/package.py` and `scripts/release.py`: build a complete archive and
  verify it into a new release directory.
- `web`: the application interface.

One Cargo workspace and lockfile cover both applications. The default target is
browser-based. Native depends on the shared libraries; those libraries do not
depend on native. Their browser routes do not select a native provider.

Native device admission is a separate `framer-native::account::device_owner`
contract. Its grants and authenticated SDK proofs live with the native account
code. Native serving receives this owner explicitly; the browser bridge no
longer exposes native enrollment, mint or cleanup methods. Request cleanup
still retains the supplied owner until both transport and worker have joined.

The file session store lives in `framer-store/file_sessions.rs`. One process
owns a session directory with an OS file lock for the store's whole lifetime,
matching the journal's profile ownership. Different paths to that directory
do not create independent owners. The lock file stays in place across exits;
kernel lock ownership, not its presence, determines whether startup can proceed.
Session replacement uses a private temporary file and an atomic rename.

Native generations retain the store with their HTTP client. Credential
preparation also retains that client until its worker ends. Closing the store
bridge or dropping a response waiter therefore cannot release profile ownership
while that work is still held. The lock is cooperative: old binaries do not
honor it, and it does not replace the journal's proof of prior-process cleanup.

The old parallel checker and combined-mode startup/routing runners are removed.
Their [validation record](history/parallel-browserless-validation-before-reorganization-20261025.md)
is historical. Browser/shared Python tests live in `runtime/tests`; native
Python tests live in `runtime/tests_native` and run in the native CI job.
Rust tests stay with the libraries whose behavior they cover.

Legacy host source patching, one-use restoration packages and their combined
application/worker fixtures are also removed. Build the browser package with
`scripts/package.py`; its runtime uses the journal and shutdown path described
below. The old setup and measurements remain in the history, with immutable
source links where relevant.

The browser archive carries its matching host and UI. The executable resolves
both from its own package, independently of the working directory. Development
overrides are explicit command-line options. Native archives carry their ledger
runtime and standalone service. See [application packages](releases.md).

Concurrency-limit storage is selected by application startup. Library gates
are in memory unless supplied a state-file path. Fixtures never inherit the
normal service's learned limits through an implicit home-directory lookup.

The browser host exposes only browser commands. Ordinary account activity and
pending RPC ownership live in `accounts/`; native observation and comparison
adapters are absent from both the Rust bridge and the Python browser package.
Native `LedgerOwner` uses the same journal and rate policy without importing
the browser host. It opens only existing account history and its saved primary
device, serializes mints per account, and commits charges before returning
admission. Grants remain owned until confirmed cleanup. Failed writes and
uncertain cleanup prevent reuse; closing storage cannot mark them clean.
The old native helpers coupled to host restoration and observation are removed.

This owner controls ledger access, not login. Standalone startup owns the
session directory and ledger before accepting requests, verifies the declared
worker files, and requires existing migrated history and selected accounts.
It serves ledger-backed V3 only. It does not supply V2 SDK proofs or replace
the native library's authenticated proof requirements.

The Rust ledger adapter starts `runtime/native.py` explicitly and retains the
session store until that child exits. Its private pipes carry device operations,
not session cookies. Abandoned open or mint-admission replies are cleaned up by
the connection owner; loss of the connection reports uncertainty. Shutdown
requires both a clean ledger reply and the observed child exit. The native CLI
connects this owner to the worker and the loopback complete-JSON endpoint.
SIGTERM and Ctrl-C drain requests, close native sessions and join the ledger.
Ledger and worker processes use their own process groups so terminal signals
reach this coordinator first. Startup and package checks use synthetic state;
they do not establish successful upstream browserless delivery.

Native session ownership is constructed from a session store and an operation
tracker. It retains both for its lifetime and drains its own tracker during
shutdown. Requests must use that exact pair. The session component does not
receive or mutate browser application state.

Native serving has its own state and admission policy in `framer-native/service`.
It accepts a fixed set of 1 to 64 account identities, with one logical slot for
each selected account. There is no browser status probe, browser eviction or
account-management route in this service. Identity, proxy, capacity, cooldown
and cleanup checks remain in the native admission path. The browser server's
old native control and comparison-reservation entry points are removed.

The host opens one SQLite journal outside the release directory. Account rate
decisions live in `framerize_accounts/rates.py`: request charges and mint reservations
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

A completed browser-status read can report an unavailable account without
leaving request work unfinished. That account retains its occupied, fenced
slot; other accounts can continue. Failed or mismatched reads, timeouts and
unconfirmed closes still carry unresolved cleanup evidence. `/api/health`
returns 503 while the operation gate is sealed or unconfirmed, retaining the
PID header. This endpoint neither clears the gate nor guarantees upstream
account availability. Reporting failure does not itself restart the service.

Earlier investigation notes are preserved in
[architecture history](history/architecture-before-reorganization-20261025.md).
