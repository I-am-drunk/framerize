# Connected worker memory measurement

Recorded on 25 October 2026 against application base `deeae729` with only
the `worker-smoke` measurement executable changed. All 1,263 other runtime
inputs match the completed native-network qualification. No serving code,
SDK input, account, service or deployment changed.

The synthetic operation completed and discarded its output. Its worker
exited and joined. The measurement unit also exited with no remaining main
process. This is local execution, not authenticated upstream delivery.

| Measurement | PSS | RSS |
| --- | ---: | ---: |
| JavaScript worker, sampled peak | 118.153 MiB | 122.273 MiB |
| Rust measurement process, sampled peak | 3.753 MiB | 5.926 MiB |
| Sum within an accepted sample, peak | 121.395 MiB | 127.566 MiB |

There were 41 accepted samples, zero incomplete samples and at most two
processes. Sampling ran every 50 ms plus read overhead. The Rust process
includes the sampler thread. The worker includes its SDK, JSDOM and worker
threads; threads are not counted again as separate processes.

Each metric's maximum may occur in a different sample. The combined peak
is calculated from each sample's sum, not by adding independent peaks.
Reads verify process start times and unchanged descendant membership.
Missing, malformed or inconsistent reads count as incomplete, never zero.
Sampling can miss short-lived processes and peaks between observations.
The reported numbers are sampled lower bounds, not instantaneous maxima.

The operation took 2.225 seconds with sampling enabled. This includes
measurement overhead and is not a speed comparison. The original private
network/filesystem, syscall restrictions, 128 MiB cgroup memory limit,
zero swap and 32-task limit were retained.
