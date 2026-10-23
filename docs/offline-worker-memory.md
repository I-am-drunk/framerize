# Local worker memory measurements

On 2026-10-23, six isolated runs tested the SDK worker with a synthetic
project and device. No account, session, browser service or upstream request
was used. Locally generated tokens were discarded without writing their
contents. Production retains its accepted 96 MiB old-heap setting.

Each run had a 128 MiB process-group cap, 32-task cap, no swap and a 45-second
deadline. The worker's socket, process-creation and io_uring restrictions
were retained and checked before Node started. Both account directories and
the service-control socket were absent from the filesystem view.

| Setting | Local mint | Group peak, MiB | Reported process RSS peak, MiB | Minter elapsed, ms |
| --- | --- | ---: | ---: | ---: |
| Existing 96 MiB old heap | Passed | 71.54 | 116.78 | 1184.3 |
| Explicit 1 MiB semi-space | Passed | 71.51 | 117.00 | 1239.2 |
| Optimize for size | Passed | 71.85 | 117.58 | 1327.5 |
| Experimental 64 MiB old heap | Passed | 71.82 | 117.14 | 1210.1 |
| Experimental 48 MiB old heap | Passed | 72.99 | 118.64 | 1248.6 |
| Experimental 32 MiB old heap | No result, exit 134 | 66.05 | Unavailable | Unavailable |

These are single samples, not a latency benchmark or evidence of a
statistically significant difference. The explicit semi-space flag did not
change the observed 2 MiB new-space allocation. None of the successful
variants reduced reported peak RSS. The failed run is not a memory-saving
result. Its stderr was suppressed, so the exit code alone does not prove
which internal failure occurred.

Group memory includes the measurement harness and pages charged to that
group. Process RSS also includes shared mappings, so these columns cannot be
subtracted to claim a saving. The RSS high-water mark was read inside the
worker after minting, before final output and exit. Group counters were read
again after the worker joined. These measurements do not include a running
application server or establish total loaded-service memory.

All six groups recorded zero swap and memory events, joined their workers,
and were absent afterward. The five successful runs retained device storage,
source identity and the original SDK cleanup checks. Smaller-heap trials
changed only matching heap literals in frozen launcher and worker copies.

The probe is [measure_offline_worker.mjs](../scripts/measure_offline_worker.mjs).
It requires the isolated filesystem, network namespace, syscall restrictions
and exact resource caps before loading the worker. It records numeric memory
and time observations and selected cleanup fields. It does not accept account
input or operate either service.

The saved source, input and output bindings were checked locally. Independent
review remains pending. The next useful optimization target is source-level
allocation during module loading, rather than enabling an unproven heap flag.
Authenticated streaming, interruption and loaded-service measurement still
require the separately owned account and fresh isolated runtime.
