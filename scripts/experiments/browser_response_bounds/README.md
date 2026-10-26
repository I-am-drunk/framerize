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
