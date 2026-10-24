# Rollout control must stop generating model traffic

A 30-second socket observation on 24 October attributed continued local
8787 requests to the shared Codex backend. That process is also the parent
of this investigation's command runner. The saved provider configuration
points to this same endpoint. This establishes a shared control dependency,
not attribution of every request to an individual thread.

Hold delivery and snoozing alone did not produce endpoint quiescence.
Repeated model turns can keep the service busy or extend an account cooldown.
The root must therefore stop generating model requests during the transition.

A separate scheduler waits for a declared future window, then requires the
existing positive quiet checks and at least ten minutes left in that window.
It invokes the unchanged authentication helper and bound rollout launcher.
It stores no password, uses only the terminal's existing sudo authentication,
and refuses expired authentication. The original controller retains its own
resource caps, input checks, quiet checks, state preservation and rollback.

Nine inert scheduler checks passed under the existing isolated 128 MiB,
zero-swap, 32-task runner. The process, unit and cgroup were collected.
This validates scheduling refusals and call ordering, not a real rollout.
The normal service and both live stream checks remain untouched at this point.

The coordinated execution window is 23:42 through 23:57 UTC. Its preparatory
hold is separate from that 900-second interval. Actual runtime checks remain
required regardless of delivered hold notices. T3 replacement remains held.
