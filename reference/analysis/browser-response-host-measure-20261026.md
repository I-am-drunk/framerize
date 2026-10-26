# Integrated host response measurements

The actual host's acknowledged response path reduces sampled peak memory in
this local comparison. It does not establish a material latency improvement.
The [data](browser-response-host-measure-20261026.json) includes every trial,
PSS sample, source identity and calculation input.

`host_measure.py` loaded the full-body runtime from immutable `5bfe52c975b78f7e`
and the integrated runtime from immutable `7075485e1f049cbd`. Both used the same
fixture, browser configuration and synthetic 8 MiB response. Only the request
origin was replaced. The probe checked the actual imported host's identity.

There were three memory trials per implementation with mode order alternated,
then three separate timing controls per implementation without the sampler.
Each trial had a fresh browser and cgroup. All twelve passed: exact body,
status and usage, one HTTP request, job removal, account activity release,
acknowledged browser close and descendant exit.

| Median measurement | Full body | Acknowledged chunks |
| --- | ---: | ---: |
| Sampled peak process-tree PSS | 674.653 MiB | 638.521 MiB |
| Per-trial peak growth above the pre-request snapshot | 184.071 MiB | 154.147 MiB |
| Pre-request process-tree PSS | 490.582 MiB | 484.279 MiB |
| Post-request process-tree PSS | 622.192 MiB | 632.767 MiB |
| Request time without sampling | 1345.842 ms | 1339.182 ms |

The difference between median sampled peaks is 36.132 MiB, or 5.356% of the
full-body median. The three peak ranges were 666.368–678.359 MiB and
632.767–653.578 MiB respectively. Per-trial peak growth subtracts each trial's
own starting PSS before taking the median; its reduction was 29.924 MiB.

Unsampled request times ranged from 1274.577 to 1364.953 ms for full bodies
and 1313.630 to 1411.275 ms for acknowledged chunks. Their median difference
was only 6.660 ms (0.495%), with overlapping ranges. The post-request PSS
values also overlapped substantially and establish no retained-memory win.

The timed operation includes the actual host's polling cadence, response
transfer, final acknowledgement and mandatory cancellation call. It excludes
browser startup, before/after snapshots and the subsequent content hash.
PSS sampling raised the median measured operation to 1822.610 and 1903.991 ms;
those figures are kept separate from the controls without sampling.

PSS totals include the Python harness, driver and browser descendants.
The sampled peak is the maximum of the before, during and after observations;
it may miss a short allocation. Cgroup peaks include startup and are recorded
separately. Three trials per group are descriptive, not a confidence interval.

All trials used browser `152.0.4-beta.31`, private network/PID namespaces,
the shared execution lock, 2 GiB with swap disabled, a 512-task limit and
the same two-CPU quota. There were no memory or task-limit events. All 120
source pins matched after execution, and all twelve units were inactive
with no remaining main process or cgroup. Measurements ran on October 26,
2026, between 01:18:45 and 01:19:48 UTC.

The [fixture instructions](../../scripts/experiments/browser_response_bounds/README.md)
describe `--runtime`, `--label` and the separate `--sampled` mode. Its label
selects reporting only; the explicit runtime directory selects the code.
The result file preserves the runtime Git trees and file hashes for both
versions, including the unchanged browser/account ownership implementation.

This is a modest peak-memory reduction in local response handling. The
roughly 484 MiB pre-request footprint still dominates this configuration.
Neither implementation here runs account startup, SDK collection or the
native minter. No real Framer request or deployment occurred, and the result
does not establish a working browserless or hybrid mint-to-request path.
