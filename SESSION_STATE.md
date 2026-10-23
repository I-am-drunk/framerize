# Admission failure checkpoint

Updated 2026-10-23. Branch: `fix/live-admission-503-20261023`.
Base: `bd246222f7a3b986d05c542cf4f2d83e408198bb`.
Top-level T3 thread: `mcp:28c55101-e602-443c-a616-db31e42ee907`.
Model remains `gpt-6-astra-ultrafast`, reasoning effort `xhigh`.
Persistent goal is active with no token budget.

Objective: correct the real normal-endpoint admission 503, deploy through one
runtime owner, and prove normal requests and post-disconnect admission recover.
No broader browserless implementation or test cleanup is in scope.

Runtime owner: T3 thread `544711e3-0b76-4cb4-9732-124a0486847e`.
Coordination request sent before implementation or live actions. No runtime
ownership has been transferred. This thread has made no live service queries.
R11 terminal 31590 refused read-only readiness with `service generation changed`.
The quiet hold is released; no R11 deployment, park or second inference ran.
R9 binary `72ab248a...` and PID 2243582 are historical, not current proof.

Read: repository AGENTS.md, architecture, development status, owner checkpoint.
No source modification, build, regression or deployment has run in this thread.

Owner confirmed this thread owns admission source and regression preparation.
Owner retains the serial build slot and all service/account/request actions.
Saved diagnosis SHA256 `9a861be6f087bd8ba489b2eca8e2cc03ce7a15c23ed26be9fdf73ed1cc21e408`
was verified. At 03:32:48 UTC health/pool were 200 (11 available, 6 residents).
Current main PID 2270461, invocation `5059ae9aa6cf420e907b7176f3959833`,
started 03:28:49 UTC. Binary is still `72ab248a...`, host `dfcdd440...`.
The preceding generation's failure cause is not established by those 200s.

Source correction prepared: assembled nonstream Responses carry completion
evidence; unread completed bytes no longer imply unknown external work.
HTTP waiter disconnect reaches ChainConfig; an existing call stays owned and
future attempts stop. Unknown cleanup and raw/paid streams remain fenced.
Safe refusal events distinguish Full, Sealed and Unconfirmed.
Three routed cases cover dropped success, disconnect with success/refusal and
unknown cleanup on body drop/consumption. Existing API cancellation evidence
is reusable because no core, host, API or store code changed.
Diff checks pass. rustfmt parsing was attempted but its component is absent;
no compiler/test/installation ran here. Owner holds the serial build slot.
Saved journal has no captured failed request, so exact incident attribution
remains unknown. The concrete source bug is independently reproducible.
Exact next action: commit/push the reviewed source, then send the owner its
commit, four source paths and three case names for bounded validation/build.
