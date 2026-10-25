# Extracted V3 wrapper control qualification

Nine selected cases and 35 leaf scenarios passed on 25 October 2026.
Node counted 44 tests including their parents, with no skips or cancellations.
The final fixture is `0a9d77120b15976a8426c73eb49476547602cd32a222e2d3963a3a79e75fb995`.
Source binding is `45ae67b2fe099210353c7f6983ae1478bfcbf64ae920bbdbc6d9fdff96fb45fa`.

The fixture evaluates the frozen wrapper's bytes [553630,555775) and timeout
helper bytes [0,103). It excludes the preceding bundled V2 initializer and
replaces only the selected V3 import expression with an inert loader. The
2,325-byte composition hashes to
`4bfbcd71280593a44030b575f07911e27c7d4e0c6248d1423c4a6936fdddebf9`.
Complete SDK files are identity inputs, never evaluated.

The observed wrapper behavior distinguishes caller wait expiry from guarded
phase failure. A caller can fall back while cached initialization continues.
Guarded failure disables V3; strict-null fallback treats undefined separately.
Returned results, callback errors and timeout do not cancel other pending work.
The fixture explicitly settles its own pending work and drains its virtual timers.

Execution used Node 20.20.0 with a 64 MiB heap limit, private PID/network
namespaces, read-only inputs, 128 MiB memory, no swap, 32 tasks and one CPU.
All 16 bound inputs were unchanged. The job completed with exit zero; its unit,
cgroup and both recorded launcher/client PIDs were absent afterward. The shared
lock was free. Transient descendants were not exhaustively recorded.

Result: `340c742dfaaf393fc64fc57d35b69b9d70ee3f799c92a68dcebfc1a20d51c7f7`.
Collection: `e719a14bf370eeeda3403cd906b1b744f20e8e226c4c0e46b034039b5cf1b90f`.
Nonthrowing diagnostics and virtual clock ordering limit these observations.
They do not establish real SDK behavior, native acceptance or real SDK cleanup.
The independently qualified normal-service rollout remains at its specific
service-approval boundary. No service action or live request ran here.
