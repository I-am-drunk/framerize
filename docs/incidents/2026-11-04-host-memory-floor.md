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
