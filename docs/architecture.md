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
require confirmed cleanup before reuse. Graceful service shutdown, automatic
crash recovery and first migration remain unfinished; this code is undeployed.

Earlier investigation notes are preserved in
[architecture history](history/architecture-before-reorganization-20261025.md).
