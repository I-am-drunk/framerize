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
The probe calls the minter directly. It does not exercise the Rust worker
owner, private token-pipe handoff, native HTTP path or authenticated delivery.

All six groups recorded zero swap and memory events, joined their workers,
and were absent afterward. The five successful runs retained device storage,
source identity and the original SDK cleanup checks. Smaller-heap trials
changed only matching heap literals in frozen launcher and worker copies.

The probe is [measure_offline_worker.mjs](../scripts/measure_offline_worker.mjs).
It requires the isolated filesystem, network namespace, syscall restrictions
and exact resource caps before loading the worker. It records numeric memory
and time observations and selected cleanup fields. It does not accept account
input or operate either service.

Independent review `c87b7653` accepts the original six-run source and saved
evidence, with no findings. Verification `948f7f8c` binds the unchanged freeze
`9d45d1b0` and summary `dc39951c`. This acceptance does not cover the later
allocation and cache studies below or authorize a runtime change.
Authenticated streaming, interruption and loaded-service measurement still
require the separately owned account and fresh isolated runtime.

## Allocation attribution

A later instrumented synthetic mint retained the original device and cleanup
checks and recorded nine numeric phase samples. Anonymous process memory
increased from 12,587,008 to 37,629,952 bytes while importing JSDOM, before
creating its window. That 25,042,944-byte increase is the largest early step
in this sample. Wrapper evaluation and minting added further allocations.

The instrumented run's group peak was 75,800,576 bytes. Its worker joined,
its unit and cgroup disappeared, and swap and memory events remained zero.
Instrumentation can affect timing and allocation. The sample identifies a
target for investigation; it does not establish an achievable saving.
Only frozen measurement copies contain these probes. Production is unchanged.

## Static compile-cache comparison

A separate preparation imported JSDOM without invoking the SDK and generated
633 static cache files totaling 2,457,612 bytes. Three subsequent measurements
mounted that cache read-only. All cache hashes remained unchanged. Only the
measurement launcher enabled caching; production worker bytes stayed exact.

| Pair | Baseline mint, ms | Cached mint, ms | Baseline group peak, bytes | Cached group peak, bytes |
| --- | ---: | ---: | ---: | ---: |
| 1 | 1184.281 | 1136.845 | 75,018,240 | 79,147,008 |
| 2 | 1199.040 | 1118.411 | 74,887,168 | 79,642,624 |
| 3 | 1177.149 | 1159.043 | 75,149,312 | 77,553,664 |

Across these samples, average minter time fell from 1186.823 to 1138.099 ms,
about 4.1%, while average peak group memory rose about 5.0%. The small timing
change does not justify adopting a memory regression. Production caching
remains disabled. Three comparisons do not establish a reliable latency gain,
and cache-hit telemetry was not collected. No live request latency was measured.

The attribution, preparation and comparison runs passed their original caps,
device checks where applicable, and teardown checks. Saved evidence was
verified locally in follow-up summary `5a1b30b6`, reusing the first baseline
from the accepted six-run study. The 1,247 installed/tool declarations were
forwarded without reading their targets. These follow-up results have not
received independent review. No experiment was rerun to publish this summary.

## Bounded verification buffers

One additional synthetic run replaced whole-file hash buffers with a reusable
64 KiB buffer. The input inventory, expected digests, module parsing, SDK,
DOM, device and cleanup checks stayed unchanged. This changed only the
experimental source copy.

The local mint passed, but did not demonstrate a useful saving. Peak group
memory was 75,436,032 bytes versus the saved baseline mean of 75,018,240.
Reported process RSS peaked at 123,359,232 bytes. Minter time was 1209.069 ms
versus the baseline mean of 1186.823 ms. One sample cannot establish a small
regression, but it provides no reason to adopt the change.

The worker joined, the unit and cgroup disappeared, and swap and memory events
were zero. Saved comparison `2bf821c0` records the result. It has not received
independent review. The [experimental patch](../reference/analysis/offline-worker-verification-buffer.patch)
is preserved for reproduction and is not applied to production.
This led to the import-only diagnostic below.

## Import-only allocation diagnostic

The [import probe](../scripts/measure_offline_imports.mjs) ran once with the
same isolation and resource caps, without invoking the SDK. It observed
2,546 loader calls and 638 uncached module loads, then restored its loader
hook. Import time was 475.378 ms. Peak group memory was 41,947,136 bytes;
reported process RSS peaked at 82,399,232 bytes. This smaller workload cannot
be compared with the full mint to claim a memory saving.

The largest observed single-module net heap increase, after subtracting
nested observations, was 6,966,264 bytes around
`cssstyle/lib/CSSStyleDeclaration.js`. Package aggregates were 10,828,496
bytes for jsdom and 7,709,672 for cssstyle. Some other observations were
negative because collection and measurement overhead affect these boundaries.
The values locate candidates; they do not assign exact ownership of memory.

Source inspection finds eager property-descriptor installation on the CSS
style prototype at this point. The subsequent experiment below investigates
allocation caused by repeated prototype changes. No dependency change has
been applied to production.

All inputs remained exact. The worker joined, its unit and cgroup disappeared,
and swap and memory events were zero. Summary `89f3ecc3` records the result;
it is locally checked and has not received independent review.

## CSS prototype construction experiment

A separate dependency copy adds and removes two temporary symbol properties
before installing the original CSS properties. The original dependency stays
unchanged. The [fixture](../scripts/measure_offline_css.mjs) uses an explicit
read-only overlay and checks its actual source hash.

The first pair matched all 1,553 final property descriptors and the hashes of
1,537 per-setter observations. These compare the two implementations with the
same inputs; they do not establish comprehensive CSS correctness. Group peak
fell from 49,573,888 to 43,384,832 bytes in this import-and-fixture workload.
Summary `5a262d7f` preserves this limited result.

One full synthetic mint then used the same experimental overlay with all
seven worker modules unchanged. The measurement child checked the overlaid
file's hash before and after minting. The original SDK, device and cleanup
checks passed. Group peak was 74,878,976 bytes, only 8,192 bytes below the
lowest saved baseline sample, with reported process RSS of 122,822,656 bytes.
Minter time was 1330.950 ms against the baseline mean of 1186.823 ms.

The apparent import saving did not produce a useful full-worker result.
The [dependency patch](../reference/analysis/offline-worker-css-layout.patch)
is preserved but remains unapplied. All three workers joined, their units
and cgroups disappeared, and swap and memory events were zero.
Full-mint summary `c46984c2` and the fixture summary are locally checked,
not independently reviewed. No service or real account was used.

Further measurement should cover the real worker entry and private pipe
handoff under its owner. These direct-minter experiments do not measure that
integration. Existing owner tests and saved validation should be checked
before adding a focused measurement, to avoid repeating established results.
