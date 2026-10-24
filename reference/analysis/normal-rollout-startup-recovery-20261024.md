# Normal-rollout startup recovery qualification

The normal endpoint repair remains distinct from ordinary-V2/native acceptance.
This milestone prepares recovery for the already-authorized normal rollout.
It does not deploy the repair or switch browserless behavior onto main.

The controller closes an owner-bound positive ConditionPathExists permit before
changing state ownership. It verifies the loaded condition, drop-in identity,
restart policy and exact startup argv. Failure cleanup closes the permit.
An unrelated live executable is refused before any recovery stop.

Unused-state recovery requires the original never-resumed termination receipt,
exact boot/process/snapshot/whole-inventory bindings, successful service stop,
no queued job, no remaining scope members and two checks during one continuously
closed fence epoch. Any claim, restored, accepted or temporary marker refuses
reuse. Reassignment archives the original request and preserves snapshot bytes,
source identity, capture time and inventory while changing only the destination,
nonce and snapshot reference. Publication errors never authorize a start.
A restored candidate follows the separate fresh-capture rollback path.

58 focused checks passed in five isolated jobs with 128 MiB memory, no swap,
32 tasks and no private runtime mounts or network. Result prefixes are
5bdc7f90 (41), 22b2d6fb (6), 885c665c (1), a0f9f44d (5), and 14178936 (5).
Every launcher/direct-client PID and owned unit/group was collected; the shared
lock was free. Transient descendants were not exhaustively captured.
The tests do not establish actual service fencing or real state restoration.

Plan 9f3c55fc binds the unchanged 15-file release with binary 314c3190 and host
f94e2d48, 61 source/request pins, and the original normal service generation.
The terminal helper supplies only the bound read-only memory descriptor after
local administrator authentication. That authentication is currently missing.
Fresh client coordination, positive endpoint quiescence and independent recovery
control must still precede the live transition and same-generation stream pair.
No live request or service mutation occurred during this milestone.
