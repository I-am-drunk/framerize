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
Commit `f4bb927136f2add85c52dff7913ebff6d177b1a5` is pushed to the matching
remote branch. Owner received four source paths and exact three case names.
Focused scope is the original 16-case cancellation slice plus three new cases
(19 total). The full ownership module alone has 14 original cases, not 16.
The owner's first compile passed (peak 972304384 bytes, clean controls).
The first 19-case run passed 18; the unknown-cleanup case timed out waiting
for an unanswered second-account call. Failure evidence remains immutable in
the owner's `investigation/admission-repair-r1` directory.
The additive correction supplies that existing failover reply and carries
cleanup uncertainty across later successful attempts. This prevents the new
completion proof from forgetting an earlier account's unknown cleanup.
Additive correction `fc774556aca127801bc8b307d370ff1e1d691d57` is pushed.
Owner integrated it into browserless and is publishing a one-line
`ai.post.started` event after successful AI_START_JS evaluation. This supplies
actual call-start observation for the live disconnect check.
The release preserves exact R9 six-file support manifest `b88742d6...`.
No R11 native-device support delta or offline trial is included. Worker-config
bytes stay identical; an owner-staged path change will be explicit.
Exact next action: collect replacement focused results and the owner's
combined build/deployment identities. No local repeat or live action occurred.

Review found a retry-admission early return that bypassed accumulated cleanup
uncertainty. The additive correction attaches the existing error response to
that same proof, retaining status/body/retry headers. Serialization errors
also retain the proof. The existing focused case now holds a replacement
account's permit, marks it cooling, and consumes the resulting refusal.
Owner's `admission-repair-r2` snapshot is preserved unrun. No replacement
compiler has started. Exact next action: publish this correction for the
owner's final narrow review and replacement validation.

Live acceptance remains pending. Use the normal `/openai/v1/responses` route,
with one bounded nonstream request disconnected after its current call starts,
then a bounded successful request after observed completion. Keep one runtime
owner, original cooldowns, and the same service generation across both checks.
Record current PID, invocation, binary and support identity with sanitized
completion/status/cancellation evidence. A restart or pool 200 is insufficient.
