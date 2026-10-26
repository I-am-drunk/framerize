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
