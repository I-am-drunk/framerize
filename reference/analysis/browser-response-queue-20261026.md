# Browser response queue experiment

Recorded on 2026-10-26, on base
`06bcbf242c4290f8137ad93d0e2e5dbefb764d84`, with browser `152.0.4-beta.31`.
The [result data](browser-response-queue-20261026.json) retains each trial,
its timing, memory observations and source hashes. The branch is
`fix/browser-response-bounds-20261026`; this work is undeployed.

This experiment drains decoded response text while the browser fetch is
still running. It preserves the production request job, fetch and cancellation
handler, replacing only the response reader and the host polling operation.
The original production reader and poll remain unchanged.

The text queue holds at most 131,072 UTF-16 units. The producer waits when
adding another decoded chunk would exceed that bound. Draining the queue
wakes it; aborting also wakes it and discards pending text. The existing
8 MiB received-byte limit remains. The host still assembles one final body.

This bounds queued decoded text. It does not bound browser network buffers
or the raw reader chunk still being decoded. No account, SDK or upstream
request is involved. The native-minter/browser connection remains separate
and unqualified by this experiment.

The first real-browser failure check passed: the producer waited at the
128K-unit bound, cancellation joined both that blocked producer and a fetch
waiting for network data, and an oversized body retained its failure phase
and HTTP 200 head while closing the peer connection. All request jobs were
removed and all browser descendants exited. The result is a local fixture
qualification, with synthetic credentials and a temporary browser profile.

The comparison includes request start, fetch, driver calls and final Python
concatenation. A concurrent sampler records process-tree PSS, waits 50 ms
and repeats. Sampling cost extends that interval and is included in timing.
It can miss short-lived peaks.
Kernel cgroup peaks use separate accounting and include browser startup.
These timings cannot be compared directly with the earlier
[completed-body copy timings](browser-response-transfer-20261026.md).

Sampling took 0.43 to 0.67 seconds per trial, so a separate comparison through
`queue_timing.py` removes concurrent PSS reads. The memory and timing controls
retain the same reader, polling functions, browser settings and body checks.

All six memory trials passed, with three fresh browsers per mode. The
median sampled PSS peak fell by 12.828 MiB (1.9%); the median cgroup peak
fell by 26.281 MiB (4.6%). This is a modest improvement in this fixture.

| Mode | Sampled PSS peak, MiB median (range) | Cgroup peak, MiB median (range) | Instrumented request time, ms median |
| --- | ---: | ---: | ---: |
| Full body | 664.552 (658.619 to 675.490) | 565.555 (561.668 to 568.902) | 1457.8 |
| Bounded queue | 651.724 (638.616 to 658.854) | 539.273 (534.957 to 547.082) | 1119.7 |

PSS includes the harness, driver and browser. The peaks include snapshots
before and after the request as well as the concurrent samples. They are
observed maxima, not continuous process peaks. Cgroup peaks include startup
and use different shared-page accounting. Three trials do not establish a
production memory distribution or a confidence interval.

All six timing controls also passed. Without concurrent sampling, median
request-to-body time fell from 1121.7 ms to 799.7 ms (28.7%). Full-body
trials ranged from 1071.1 to 1223.6 ms; queued trials from 776.9 to 812.8 ms.
The local 8 MiB fixture, browser startup policy and final body checks matched
between modes. These are fixture timings, not a live request speedup.

All thirteen browser runs used fresh temporary browsers, private network
and PID namespaces, 2 GiB/no-swap cgroups, 512-task limits and the shared
execution lock. All owned units ended inactive/dead with MainPID 0.
There were no resource-limit events. The 25 memory-run source pins and
26 timing-control pins matched at their post-execution checks.

The twelve Node contract cases subsequently reproduced an absent-body
cancellation defect: eleven passed, while an already-aborted response with
no body incorrectly succeeded. The reader now rejects that branch; all
twelve cases pass. They cover backpressure, UTF-8 boundaries, exact byte
limits, joined cancellation, status/usage and unrelated-job preservation.
Both contract units exited and were collected, with no process remaining.

All browser measurements above used reader SHA256 `b23e17bd...`, recorded
in full in the result data. The corrected reader is `8e74077e...`; only its
absent-body branch changed. The browser measurements were not repeated or
relabeled. The poll and delivered contract fixture retain their original
bytes, and the corrected source has Node qualification only.

The next integration dependency is acknowledged, retryable chunk delivery.
The experimental poll consumes text immediately. If its RPC reply is lost
or times out, another poll can skip that text. Production integration must
retain each chunk until the receiver acknowledges it, including the final
chunk, while preserving the account owner, byte limit and joined cleanup.
The current performance results do not qualify that delivery contract.
