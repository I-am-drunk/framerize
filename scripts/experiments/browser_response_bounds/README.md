# Browser response fixture

This experiment runs the production browser AI request scripts against a
local HTTPS server. Only the target origin is substituted. Credentials and
bodies are synthetic; no SDK, account, project or upstream service is used.
The browser launches directly through Playwright in a temporary profile.

It checks an intact 8 MiB response, rejection above that limit, and cancellation
of a response that remains open. Rejection and cancellation must close the
server connection. The browser job table must be empty afterward, and all
browser descendants must exit before the experiment succeeds.

Memory samples include the harness, driver and browser. They are snapshots,
not continuous PSS peaks or production memory savings.

Use the namespaces, 2 GiB/no-swap cgroup, shared execution lock and pinned
runtime from the [shared-context fixture](../shared_context_isolation/README.md).
Mount the repository root at `/source` instead of mounting that fixture at
`/work`. Set the working directory to `/source` and run:

```text
<fixture-venv>/bin/python -I -B \
  /source/scripts/experiments/browser_response_bounds/probe.py \
  --browser /browser/camoufox
```

Keep both `FIXTURE_PARENT_NET` and `FIXTURE_PARENT_PID` bindings. The probe
refuses the original namespaces. The self-signed certificate exception is
limited to this isolated, temporary browser context.

## Completed-body transfer comparison

`transfer.py` uses the same fixture and browser settings. It waits for the
exact 8 MiB ASCII body, then transfers it with the production `AI_POLL_JS`
or the experimental `transfer.js` reader. Run it in the same isolation with
`--mode full`, `--mode 32768` or `--mode 131072`, plus the browser argument.
Each invocation needs its own fresh browser and cgroup.

Chunk sizes count UTF-16 units. The reader keeps surrogate pairs together,
checks sequential offsets and removes the job only after its final chunk.
The Python receiver joins all chunks into the same complete response body.
This compares driver copying; it does not stream responses to an application.

The timed interval includes driver calls, chunk checks and the Python join.
It excludes fetching, completion polling, memory sampling and the final body
check. Samples cover the harness, driver and browser before and after transfer.
Kernel `memory.peak` covers the cgroup lifetime, including browser startup;
it is not a PSS peak or an isolated transfer-allocation measurement.

The comparison checks exact content, status, usage, job removal and joined
descendant cleanup. Use rotated mode order across repeated fresh invocations
and retain every result, including failures. This experiment does not change
the production poll operation or establish live request savings.

Run the pure chunk-contract fixtures with
`node --test scripts/experiments/browser_response_bounds/transfer_contract.cjs`.
They exercise an isolated VM and do not launch a browser or import an SDK.

## Queue drained during fetch

`stream_queue.py` substitutes an experimental response reader into the
production start script. It retains the original fetch, job and cancellation
ownership. The host drains at most 131,072 UTF-16 units per call while the
reader runs; a full queue makes the producer wait. The host still returns
one complete response and discards partial content if reading fails.

Run `--mode checks` for full-queue cancellation, oversized responses and
cancellation while awaiting network data. Use `--mode full` and
`--mode queued` in separate fresh invocations for the comparison. All modes
require the same browser argument, private namespaces and resource limits.

This comparison times request start through arrival of the complete host
body, including fetch and driver transfer. It differs from the earlier
completed-body transfer timing. A concurrent sampler records process-tree
PSS, waits 50 ms and repeats; sampling cost extends that interval and is
included in the timing. Samples can miss a short-lived peak. Kernel cgroup
peaks remain a separate measure. `queue_timing.py` repeats the same full and
queued paths without concurrent sampling, for a separate timing control.

The queue bound covers decoded text waiting for the host. It does not bound
the browser's network buffers or the raw chunk currently being decoded.
Neither experiment uses a real account, SDK or upstream service. Neither
changes the production response reader or makes a live acceptance claim.

Run the twelve queue contract cases in the same isolation with
`node --test scripts/experiments/browser_response_bounds/queue_contract.cjs`.
They use controlled readers in fresh VMs, without a browser or SDK. The
fixture publishes a completed job only after its reader promise settles;
that helper models publication, not the entire production wrapper.

The [recorded comparison](../../../reference/analysis/browser-response-queue-20261026.md)
uses the original reader. Its later absent-body cancellation correction has
Node qualification only; the recorded browser trials were not repeated.

This poll removes chunks before the host acknowledges them. A lost RPC
reply can therefore lose content. Before production use, implement explicit
acknowledgement and retry of the same chunk, including final-job cleanup.

## Acknowledged delivery

`queue_ack_poll.js` and `queue_ack.py` add explicit sequence acknowledgements.
The pending frame retains its share of queue capacity and remains replayable
until acknowledged. A final frame also waits for acknowledgement before the
job is removed. The original measured reader and poll are unchanged.

Run `ack_probe.py --browser /browser/camoufox` in the same private namespaces
and resource bounds as the other browser fixtures. It checks a normal body,
five deliberately lost RPC replies, capacity while a frame is unacknowledged,
joined cancellation and oversize refusal. Retries must not repeat the HTTP
request. This is a correctness fixture, not a performance comparison.

Production use must retain the host's account-owned `Host._rpc` wrapper and
request deadline. This standalone receiver does not qualify that integration.
See the [recorded result](../../../reference/analysis/browser-response-ack-20261026.md).
