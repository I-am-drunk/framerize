# Local worker memory measurements

The deferred IDNA-table experiment derives all 128 ASCII mappings from the
pinned `tr46` Unicode table and loads the original table for non-ASCII input.
The original and candidate returned identical output hashes across 105,216
calls each, including every ASCII code point in three positions, all 128
option combinations, and Unicode/punycode cases. The candidate kept the
table unloaded for ASCII and loaded it when Unicode processing needed it.

One subsequent synthetic SDK mint passed with the table still unloaded.
Its group peak was 56,176,640 bytes versus the saved current-flag probe's
56,434,688. Reported peak RSS was 96,210,944 versus 96,002,048 bytes, and
minter elapsed time was 1215.091 versus 1215.059 ms. This did not demonstrate
a useful peak-memory improvement. The candidate also performs extra identity
checks, so this single comparison is not a precise performance benchmark.

The dependency remains unchanged in production. The exact [unapplied patch](../reference/analysis/offline-worker-idna-lazy.patch)
uses zero-context format. The [equivalence probe](../scripts/measure_idna_loading.mjs)
and summary `8cb638bf` preserve the results, joined cleanup, absent units and
cgroups, and zero swap/memory events. No account or authenticated request
was involved. Faster repeated ASCII conversion in the fixture does not prove
a live request-latency gain.

A current-flag allocation probe retains the same nine observation points
and adds `--jitless`. All seven worker modules still match current source
after reversing the measurement hooks. JSDOM loading added 21,561,344 bytes
of anonymous memory; at mint completion, the process had 47,112,192 bytes
of anonymous PSS and 45,242,368 bytes of file PSS. This is one synthetic
worker sample, not integrated service memory or authenticated performance.

One separate experiment requested local inspector garbage collection after
wrapper evaluation and before minting. It preserved the SDK, device state,
deadlines and cleanup checks, and disconnected its extra inspector session.
The worker environment gained no exposed GC global. Both runs passed.

| Current flags | Group peak, bytes | Reported RSS peak, bytes | Minter elapsed, ms |
| --- | ---: | ---: | ---: |
| JITless phase probe | 56,434,688 | 96,002,048 | 1215.059 |
| Plus one pre-mint collection | 55,287,808 | 94,969,856 | 1306.140 |

Collection itself took 158.083 ms. It reduced heap use immediately, but
reported peak RSS differed by only 1,032,192 bytes across these single
samples. This does not justify adoption or establish a reliable saving.
Production stays unchanged. The [reproduction patch](../reference/analysis/offline-worker-pre-mint-gc.patch)
is preserved and unapplied, with zero-context format (`git apply --unidiff-zero`).
Summary `72049ab5` binds both results, unchanged
inputs, joined cleanup, absent units/cgroups and zero swap/memory events.

A separate attribution sample binds process roles by known process starts
and executable device/inode, without reading arguments or private inputs.
It retains unknown roles in the estimate and rejects changed identities.
At the peak simultaneous non-helper sample, the observed PSS values were:

| Role | PSS, bytes |
| --- | ---: |
| SDK worker | 92,988,416 |
| Application | 25,802,752 |
| Lazy host | 20,989,952 |
| Separate owner probe | 2,342,912 |
| Sum at that sample | 142,124,032 |

Known measurement processes and fixture wrappers contributed another
16,559,104 bytes of PSS with explicit RSS fallback. No unknown role occurred
in that sample. The measurement recorded 82 complete samples and rejected
two incomplete observations. Group peak was 90,284,032 bytes with zero swap
or memory events. Fifteen focused cases and final joined teardown pass.
Summary `42afcec1` binds the result. The 2,324.180 ms synthetic owner interval
does not measure live request latency. No binaries or production paths changed.
The worker is the largest measured component; attribution itself saves no
memory. The separate owner probe and absence of authenticated HTTP remain
limits on interpreting the sum as real service memory.

The earlier combined sample (2026-10-23) includes the application server,
lazy host, separate owned worker and measurement helpers. It does not send
an authenticated request. Group peak was 89,899,008 bytes, sampled process
PSS with RSS fallback was 159,323,136 bytes, and summed process RSS was
195,010,560 bytes. The lower group charge does not prove that the complete
service fits within 100 MB. Shared mappings, charging and measurement scope
differ between these counters.

There were 83 complete process samples and one rejected membership change.
One process required RSS fallback because its detailed map was unavailable.
Eight focused tests verify that fallback is counted and that partial,
malformed or changing-membership samples are rejected. The first measurement
failed its coverage check and remains failed; a later missing output-parent
error occurred before admission and is also preserved.

The worker owner interval was 2233.986 ms, including startup and joined
cleanup. The application remained alive afterward. Final cleanup left no
unit or cgroup, with 18 peak tasks and zero swap or memory events. The
existing application and probe binaries were reused. Summary `9e644000`
binds the completed results. This single synthetic sample neither measures
real request latency nor completes authenticated streaming acceptance.

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

These direct-minter experiments do not measure the Rust owner or private
pipe integration. The following separate measurement covers that boundary.

## Actual owner and private-pipe measurement

The [Rust harness](../scripts/measure_owned_offline_worker.rs) links the exact
owner library used by the accepted application build. All five owner source
files still match that library's saved build. It runs the original worker
entry with synthetic project/device input and the original 96 MiB heap.
No account, host admission or HTTP call is involved.

The local mint passed the real descriptor, framing, metadata and freshness
checks. The harness waited for joined completion, observed the worker's
absence, and discarded and cleared the token bytes. Group peak was 75,116,544
bytes with 12 tasks and zero swap/events; the outer unit and cgroup disappeared.
Owner elapsed time was 1978.228 ms, including verification, process startup,
minting, handoff and join. SDK elapsed was 1270.783 ms; handoff was 11.345 ms.
These are one local sample and do not measure request latency or service memory.

Two build-wrapper failures are retained: a read-only compiler working
directory and an unavailable cc alias inside the isolated filesystem.
A third attempt refused before launch at the memory reserve. After source
preparation finished, the unchanged guard admitted the successful build,
which peaked at 298,430,464 bytes with zero swap/events and clean teardown.
The harness and owner source needed no correction.

Summary `c3edd067` binds the saved build and runtime results. Direct compiler
and external library inputs were checked; transitive cache and sysroot remain
trusted. Final review `089c5975`, verification `a6899afc`, accepts the exact
source and saved evidence with no findings. Omitted artifacts and installed
targets were checked by saved declarations only. This grants no runtime or
authenticated-delivery approval. Production remains unchanged.

## Actual-worker cancellation

The [cancellation harness](../scripts/measure_owned_offline_cancel.rs) retains
the original worker, owner library, isolated filesystem and resource caps.
Two distinct local cases passed: an explicit cancellation request and dropping
the owner. Before interruption, each observed the owned child alive with the
same start identity and at least ten CPU ticks. This confirms process activity,
but does not identify the SDK execution phase.

Explicit cancellation returned the expected cancellation failure. Dropping
the owner did not request a token result. Both cases joined the actual child,
observed its absence, and left no test unit or cgroup. Interruption-to-join
times were 3.268 ms and 2.896 ms in these single samples. Each used at most
11 tasks, with zero swap and memory events.

Group peaks were 27,766,784 and 27,746,304 bytes. These smaller numbers reflect
early termination and are not full-mint memory savings. No application service,
account, HTTP request or upstream model was involved. These cases do not prove
real-stream cancellation, post-cancellation admission or user-visible latency.
Summary `ba8d6ac0` binds the passing build and both saved results. The exact
tested source and saved evidence are accepted by final review `72ba6876`,
verification `1ae57d6d`, with no findings. The previous mint measurement
and its frozen review package remain unchanged.

## Combined native host and worker

The [historical combined fixture](https://github.com/diddletime/framerize/blob/3bdfb6a2984b04cae611db8f230468681587dae7/scripts/tests/run_host_worker.py) reused the accepted
lazy host and compiled owner harness. One synthetic device lease remains
held while the unchanged worker completes its local mint. Only after the
worker joins and its absence is confirmed does the fixture report completion
to the host. It releases the lease, then opens and closes a fresh lease without
another mint. The host retains the same generation throughout.

That fixture and its host-restoration setup were removed during the application
split. The measurements below remain historical evidence for those exact
inputs, not measurements of the reorganized applications.

The single run peaked at 102,887,424 bytes of group memory with 15 tasks.
Owner elapsed time was 1934.152 ms, with 1249.716 ms reported for the SDK.
Swap and memory events stayed zero. All test processes joined, and the test
unit and cgroup were absent afterward. This includes the host, worker and
measurement helpers. It excludes the application server, HTTP, authenticated
inference and concurrent requests, so it does not establish total service
memory, the full memory target or user-visible latency.

The host uses the existing Python environment with browser libraries masked.
A nested isolated filesystem keeps that environment and all browser-owned
state out of the worker's view. The original worker namespace, syscall,
device, freshness and private-pipe checks remain unchanged. The fixture's
optional device parameter preserves the previous default by exact reversal.

Summary `6af8688a` binds the saved result, 22 source inputs and 12 output
files. Final review `d98b96cf` and verification `e8dbefc2` accept the exact
source and saved evidence with no findings. No additional Rust compilation,
application daemon, real account, model request or production change occurred.

The next prepared coverage checks cancellation through the production
device-mint bridge itself. Three UNRUN cases retain the account barrier until
worker join and a controlled host acknowledgement, including an unconfirmed
reply. The worker fixture performs no SDK call. Three harness build admissions
were refused before launch; these add no runtime result or memory measurement.

## JITless comparison

One subsequent synthetic mint disabled V8 JIT compilation while preserving
the original 96 MiB heap, worker/SDK sources, isolation and cleanup checks.
It passed and left no worker, unit or cgroup, with zero swap or memory events.
The three earlier baseline samples were reused without another baseline mint.

| Measurement | Earlier baseline mean, three samples | JITless, one sample |
| --- | ---: | ---: |
| Group peak, bytes | 75,018,240 | 57,331,712 |
| Reported process RSS peak, bytes | 122,791,253 | 96,497,664 |
| Minter elapsed time, ms | 1186.823 | 1271.386 |

Group memory was 23.58% lower and reported RSS was 21.41% lower, while minting
took 7.13% longer. These measurements describe one local comparison, not a
reliable latency benchmark or a complete service saving. They exclude the
Rust owner, native host, HTTP and authenticated inference. Summary `841d7c33`
binds the saved run and reused baselines. This sample is included in the
subsequent three-sample review below.
The measurement tool now permits `--jitless`; production does not select it.

Two additional candidate-only samples checked whether that reduction repeats.
Their group peaks were 59,219,968 and 56,967,168 bytes. Both retained every
device and cleanup check, zero swap/events and absent units/cgroups. No
baseline was rerun. Across the three JITless samples, mean group peak is
57,839,616 bytes, mean reported RSS is 96,922,283 bytes, and mean local mint
time is 1230.173 ms. Relative to the saved baseline means, those are 22.90%
less group memory, 21.07% less reported RSS and 3.65% longer minting time.

Summary `b55a2a79` binds the two new results, original sample and reused
baselines. These are small samples collected at different times, not a
reliable latency benchmark. The repeatable direction supports further
integration work but proves no complete-service saving. Production remains
unchanged. Review `f68c813b` accepts the three-sample source and saved evidence
with no findings. It does not approve a production change or live request.

Two further integration comparisons use the
[owner reproduction patch](../reference/analysis/offline-worker-owner-jitless.patch).
It adds only the fixed `--jitless` argument to the real owner's launch command;
the other four owner modules remain exact. The isolated library and unchanged
owner harness both compiled. One owner-only run and one combined host/owner
run then passed using the original SDK, synthetic inputs and cleanup checks.
Their matching earlier baselines were reused without another baseline run.

| Scope | Earlier group peak, bytes | JITless group peak, bytes | Change |
| --- | ---: | ---: | ---: |
| Owner and worker | 75,116,544 | 59,097,088 | -21.33% |
| Host, owner and worker | 102,887,424 | 82,403,328 | -19.91% |

The combined case retained its device lease through worker completion, then
confirmed release and a fresh lease in the same host generation without a
second mint. Both cases joined their processes, recorded zero swap or memory
events, and left no unit or cgroup. Owner intervals were 1918.245 ms and
1911.415 ms respectively. One sample per scope, with older baselines, does
not establish a latency improvement. Summary `dd8002d2` binds both builds,
both new runs and the two reused results. Review `df13c06d` accepts their
source and saved evidence with no findings.

The combined group includes measurement helpers and excludes the application
server, HTTP and authenticated inference. It does not establish the complete
100 MB target. The experimental owner source now selects the exact measured
flag. Only isolated measurement binaries contain it so far; the application
relink is pending and the running production service remains unchanged.

The same owner variant subsequently passed the existing explicit-cancel and
dropped-owner cases after a separate harness build. Each waited for observed
worker CPU activity, then joined the worker and left no unit or cgroup. The
recorded interruption-to-join intervals were 2.865 ms and 3.117 ms, with zero
swap or memory events. The exact SDK phase was not observed, and early-exit
memory does not measure a complete mint. Summary `6a451af4` binds this separate
cancellation result; independent review is pending. No HTTP stream or real
account was exercised, and the running production service remains unchanged.
