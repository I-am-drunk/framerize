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
- `web`: the application interface.

One Cargo workspace and lockfile cover both applications. The default target is
browser-based. Native depends on the shared libraries; those libraries do not
depend on native. Their browser routes do not select a native provider.

Host module consolidation and complete release packaging are still pending.
The running service has not been changed by this reorganization.

Earlier investigation notes are preserved in
[architecture history](history/architecture-before-reorganization-20261025.md).
