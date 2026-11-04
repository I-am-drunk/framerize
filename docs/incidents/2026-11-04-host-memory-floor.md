# Alpha.30 host memory floor

Alpha.30 replaces the host reserve of one tenth of physical RAM with
1,200,000,000 bytes. Browser admission uses one Python constant; HTTP
admission and allocator reclamation share one Rust constant in framer-core.
The 1 GiB browser startup reserve, 64 MiB response reserve, 90% cgroup
watermark and installed service limits remain unchanged.

The change stacks directly on `902c100f`, retaining the alpha.29 request,
checkpoint and graceful-drain implementation. Existing accepted responses
keep their reservations and continue through later memory pressure.

Focused verification passes: four Python memory diagnostic tests, twelve
server memory admission tests and eight allocator/task tests. Boundary
fixtures cover different host sizes, negative headroom and the fixed floor.
The server tests retain streaming, reservation and tighter-cgroup coverage.

The installed external recovery guard still uses a 10% host reserve.
A prepared two-line change sets its frozen policy to 1,200,000,000 bytes.
Two private fixture tests reproduce the conflicting restart at 1.3 GB,
verify the new boundary and retain critical-memory protection. Install that
change with alpha.30, preserving the existing guard state and configuration.

The activation guard requires recent inference qualification for the exact
binary. Prior alpha.29 receipts cannot qualify a newly built alpha.30 binary.
The task forbids new inference and bypassing guards. Package build and smoke
checks can proceed, but activation must remain fenced without that evidence.
Retain alpha.29, every current database and the compatible rollback packages.

## Build and preflight result

Source `c46ef7ac801c2b10cfba9628d33e59eb753a38ac` produced the staged alpha.30.
Binary SHA-256: `54e8abc7e51da407aa5e7e6dda35e82c99bdf1668121a4c88a376371456850b9`.
Archive SHA-256: `ca54016423326d77f72d3ea68d1c1c7a85d4fa833e78dbfddc883941c8f4e306`.
All nine package smoke checks passed without inference. The package reads
checkpoint versions 1 and 2 and writes version 2, matching the alpha.29 rollback.
The preflight passed every preceding check and refused missing exact-binary
inference qualification before sending any signal. The service remains alpha.29,
PID 3231960, with all twelve identities and 404 baseline checkpoint rows intact.
Normal traffic increased the checkpoint total to 464 during the build.
The installed configuration and recovery guard remain unchanged.
The build archive, staged release, guard candidate, test logs, refusal and
preservation receipts remain private. Temporary build and smoke files are removed.

## Deployed with alpha.31

The later qualified alpha.31 package includes this unchanged implementation.
It is live as PID 3527250 from 06:17:19 UTC on 4 November 2026. The coupled
transaction installed the aligned recovery guard after alpha.29 drained cleanly.
All twelve accounts, 485 baseline checkpoints and installed settings remain.
The 06:16:50 memory snapshot reports 2,139,452,212 bytes of admission headroom;
the 06:17:49 snapshot reports 3,278,251,008 bytes with the 1,200,000,000-byte floor.
Workload also changed; the floor alone adds 467,398,860 bytes on this host.
The alpha.29 rollback, alpha.30 archive and all earlier receipts remain retained.
