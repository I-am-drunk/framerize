# Simpler applications and releases

Design decision, 2026-10-25. Implementation is in progress on the experimental
branch. This document does not describe a deployed replacement.

The project has coupled normal browser requests, native experiments, account
ownership and deployment experiments. A small change can therefore require
understanding several unrelated execution paths. Separate responsibilities
and remove the machinery for selecting both paths in the same application.

## Structure

Keep one Cargo workspace and lockfile. Default builds select `apps/framerize`,
the normal browser application. `apps/framerize-native` is a separate explicit
target backed by `crates/framer-native`. Shared libraries never depend on native.

Group native code by account, authorization, request, transport and worker.
Use one direct owned request for the existing complete-JSON endpoint. Remove
the general provider, binding map, mixed-route dispatch and fallback settings.
Retain scope validation, cancellation, independent cleanup and refusal policy.
Native streaming remains unfinished; its low-level framing code is retained.

The API library owns Framer endpoint policy and pure payload helpers. Its
browser-backed Responses chain is currently called `nativechain`; rename it
without removing its behavior. Core owns neutral account selection and pool
ownership. Server owns shared admission and the normal HTTP routes.

The initial native CLI validates configuration. It must not open ordinary
profiles alongside the running browser service. A separate executable alone
does not establish account ownership.

The native device contract belongs to `framer-native`, not `BrowserBridge` or
the shared core. Supplying this owner is an explicit native-startup dependency.
The application still needs a concrete durable owner and its startup wiring;
separating the interface alone does not provide live native admission.

## Host and tests

Extract cohesive host owners for accounts, browser operations, rate policy,
resource budgets and framed I/O. Keep mutable account history after its browser
closes. Keep package imports free of browser startup and account access.

Remove comparison tools and tests for retired mixed-mode behavior. Preserve
tests for actual protocol, rate, identity, cancellation and cleanup guarantees.
Run ordinary Cargo, Python and Node fixture tests from repository files.
Replace local scratch paths with declared inputs; do not copy private captures
or installed home directories into the source tree or build artifacts.

## Builds and updates

Build both explicit targets on pushes to the configured GitHub build fork.
Use one workflow, hosted runners and locked dependencies. No second build on
the upstream pull-request event. Each PR links its exact build and source SHA.
The build fork is a build location, not a second source of truth.

One packaging command bundles the executable, matching runtime modules, UI
and generated build information. Select the installed package canonically;
do not silently prefer a loose host script in the user's home directory.

Routine updates use one user service and `scripts/release.py activate` after
staging: validate compatibility, switch the package pointer, restart, check
local readiness and restore the previous compatible package on failure.
The command is implemented; first migration of the existing service is still
pending. Account state stays outside release directories and is never rolled
back with application files.

## Migration and implementation order

1. Separate dependencies and applications; replace native provider dispatch
   with the direct owned request. Compile both targets and run retained fixtures.
2. Produce complete browser and native developer packages through GitHub.
3. Consolidate host modules and duplicated normal request admission in distinct,
   testable changes. Replace implicit host patching with explicit adapters.
4. Implement durable rate charging before external work, crash recovery and
   graceful drain. A shutdown-only flush is insufficient.
5. Prepare one reviewed browser-only promotion onto `main`, preserving the newer
   normal repairs. Update `browser-based` only after the promotion is accepted.

The host cut is complete. Ordinary RPC ownership lives in `accounts/`;
comparison commands, evidence collection and native-device dispatch are removed
from the browser package and its Rust bridge. Actual RPC tasks outlive cancelled
waiters, and a browser cannot close beneath a pending RPC. Native authority
sources are separate and do not install hooks into the normal host.

Durable account state now uses one host-owned SQLite journal outside releases.
Request charges, cooldowns, reserve allocation, primary selection and pending
mints commit before external work. Same-boot timestamps are preserved; reboot
recovery conservatively rebases outstanding durations. Missing history on an
existing installation requires migration, not an empty ledger. The journal has
one writer, and package rollback must retain newer history.

Graceful shutdown now stops admission, joins accepted work and closes the host.
Signal handlers are installed before startup can launch a host. Failed startup
uses the same drain and close path. Confirmed browser closure records clean
account state; it does not replace writes before each charge. An interrupted
record cannot resume until its previous browser ownership has been resolved.

Recovery now uses the systemd service lifetime and kernel process membership.
Each journal row retains the runtime that last wrote it, even if that account
is not opened during an intermediate restart. New service startup must contain
only the daemon and its host, with delegation disabled and group cleanup enabled.
A different invocation of that same service can then recover dirty history.
A reboot independently proves that old processes are gone. Both paths preserve
rate restrictions and apply conservative cooldowns before further requests.
There is no external clean-state flag or custom privileged supervisor.

The first live migration is still unresolved. Today's host has no complete
ledger-export command. New persistence cannot recover old process memory.
Activation needs genuine current state or demonstrated expiry under exclusive
ownership. Producing a package does not prove either condition. No production
restart is part of this structural work.

## Examples used

- [uv's thin executable](https://github.com/astral-sh/uv/blob/0.9.5/crates/uv/src/bin/uv.rs)
- [ripgrep's application boundary](https://github.com/BurntSushi/ripgrep/blob/14.1.1/crates/core/README.md)
- [ripgrep's release workflow](https://github.com/BurntSushi/ripgrep/blob/14.1.1/.github/workflows/release.yml)
- [Meilisearch's grouped integration tests](https://github.com/meilisearch/meilisearch/blob/v1.12.0/crates/meilisearch/tests/integration.rs)
- [Syncthing's user service](https://github.com/syncthing/syncthing/blob/v1.29.0/etc/linux-systemd/user/syncthing.service)
