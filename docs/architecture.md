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
- `runtime/framerize_host`: Python browser host, with account, browser, resource,
  transport and restoration modules.
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

Removing the remaining experimental host adapters, durable account rate history
and graceful update handling are still pending. The running service has not
been changed by this reorganization.

Earlier investigation notes are preserved in
[architecture history](history/architecture-before-reorganization-20261025.md).
