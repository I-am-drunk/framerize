# Browser response transfer comparison

Measured on 2026-10-26 at 00:38 UTC, on base `20452d876c95cbec6391094e8c6a8fcf5bdd79ed`.
The experiment is on `fix/browser-response-bounds-20261026` and is undeployed.
The [machine-readable result](browser-response-transfer-20261026.json) retains
each trial, source hashes, exact times and memory samples.

All nine local browser trials passed. Each delivered the same complete 8 MiB
ASCII body, preserved status and usage, removed the request job and joined
all browser descendants. No swap or memory/task-limit events occurred.

Larger chunks improved transfer time in this fixture, without a memory win.
The 131,072-unit mode reduced median transfer time by 43.1%, while its median
post-transfer PSS was higher. The 32,768-unit mode was 20.2% slower than the
full-body call. Production `AI_POLL_JS` is unchanged.

| Transfer mode | Calls | Median transfer, ms (range) | Median post-transfer PSS, MiB | Median PSS increase, MiB | Median cgroup peak at transfer, MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| Full body | 1 | 762.9 (757.3 to 768.6) | 671.367 | 89.312 | 569.641 |
| 32,768 UTF-16 units | 256 | 917.0 (876.5 to 1062.0) | 676.975 | 94.062 | 561.852 |
| 131,072 UTF-16 units | 64 | 434.2 (423.4 to 512.0) | 690.168 | 105.166 | 578.578 |

PSS covers the harness, Playwright driver and browser at each snapshot.
The increase is relative to that trial's completed body before transfer.
These are snapshots, not continuous PSS peaks. Cgroup peaks use different
accounting, include startup and can include charges outside process PSS;
shared pages counted in PSS can also be charged to another cgroup. They are
not interchangeable measures. The smaller-chunk cgroup result alone does
not establish a working-request memory improvement.

Each mode ran three times in a fresh browser and cgroup, with rotated order:
full/small/large, small/large/full, large/full/small. Browser version was
`152.0.4-beta.31`. Every run used private network and PID namespaces, a
temporary profile, the shared execution lock, a 2 GiB memory limit, no swap,
a 512-task limit and a 60-second runtime limit.

The timed section starts after the complete body is buffered in the browser.
It includes driver calls, per-chunk checks and Python concatenation; it
excludes fetching, completion polling, memory sampling and final validation.
All modes retain a complete browser body and return a complete Python body.
No garbage collection was forced. Three trials describe this fixture, not
a production performance distribution or a confidence interval.

The experiment imports no SDK and uses synthetic credentials against local
HTTPS only. It does not qualify the native minter/browser handoff or live
delivery. The earlier response-limit and cancellation results remain valid.

All 22 recorded source pins still matched after execution. All nine owned
units were then inactive/dead with MainPID 0. Raw receipts are retained in
`investigation/browser-response-transfer-r1` in the investigation workspace.
See the [fixture instructions](../../scripts/experiments/browser_response_bounds/README.md)
for reproduction. The independent chunk-contract source fixture was queued
separately; no result from that pending assignment is claimed here.

The next memory experiment should drain a bounded response queue while the
fetch is still running, with backpressure and joined cancellation, instead
of retaining the complete body before transfer. Root owns that experiment;
these results do not justify changing the production response contract.
