# Local timing correction, 2026-11-02

Branch `fix/claude-stalls-20261101` prepares `0.5.1-alpha.12`.
The local response deadline no longer depends on output-token count. Both
chains enforce the active budget before every retry. Completed browser
responses get a separate acknowledgement window. Requested effort and
custom-only routing remain unchanged. Upstream Claude HTTP 504 at 60.122
seconds remains an external limit, not a repaired local deadline.
The 63 focused Python tests, 272 affected API tests and 95 protocol tests pass.
Both Rust return codes are zero, with fresh compilation in the iMac logs.
Production remains alpha.11, PID 2577096, started 02:03:01 UTC.
Next: build the exact committed alpha.12 source on the iMac, verify isolation,
then use the already-authorized drain/activation/rollback transaction.
See `docs/incidents/2026-11-02-local-timing.md` and cache `timing-r1`.

# Route timeout diagnosis, 2026-11-02

Branch `fix/claude-stalls-20261101`; alpha.11 remains live at source `352b4f26`.
The 180-second setting is a local buffered-response ceiling on both custom
routes. Historical code had a 900-second ceiling; native Responses requests
then defaulted to a 180-second deadline because only max_tokens was read.
No 1,080-second inference limit was found in the inspected route history.
Claude has a direct upstream HTTP 504 capture at 60.122 seconds. Recent
Responses logs also have fetch failures around 60 seconds, but no measured
upstream ceiling. Older long operation timings include preparation.
See `docs/incidents/2026-11-02-route-timeouts.md` and the private
`route-timeouts-r1` evidence directory in the Claude repair cache.
No runtime, routing or effort changes follow from this evidence. No probes
were repeated. Next: capture the next normal Responses failure's browser
status and elapsed time using alpha.11's existing observer.

# Alpha.11 activated, 2026-11-02

Source 352b4f264b7d3045aef9a2e778c6798a376a1b79 is live as 0.5.1-alpha.11.
The correction prevents SDK replay after directly returned 502/504 failures
in buffered adaptive Claude requests. All 117 affected tests and all nine
package checks passed. The iMac build took 218.705 seconds.
Activation at 02:03:01 UTC changed PID 2552520 to 2577096. Health, readiness,
OpenAI Responses and Claude Messages returned 200. All 12 account identities,
memory limits and both release inventories were preserved. Paid routes remain
disabled. No requested reasoning settings changed. The isolated service stopped.
The exact isolated custom request returned 402 from the same empty workspace.
The user's activation override was applied honestly; no isolated 200 is claimed.
Binary SHA-256: aab320ae6667d47dc2facb2dbc1ec8159aa880fe0f11a9d5fc0840e8c4ce1ce5.
Evidence: /home/tjninja/.cache/framerize-claude-stalls-20261101/visible-gateway-errors-r1.
Long custom adaptive requests remain subject to Framer's upstream time limit.
Next: inspect the next real long-Claude failure; preserve requested effort unless
the user chooses otherwise. Do not repeat paid probes or change accounts.

# Alpha.10 activated, 2026-11-02

The user explicitly said "just activate it" after the isolated custom request
returned HTTP 402. The isolated failure remains recorded, not a passing check.
Alpha.10, source 840225c8522ccc5cdd0e09aebb3bfb39d9442d3e, is now live.
Activation at 01:47:42 UTC changed PID 2340122 to 2552520.
Health, readiness, OpenAI Responses and Claude Messages returned HTTP 200.
Public HTTPS and both model-list routes also returned 200.
All 12 account identities, memory limits and both release inventories match.
Metered access is disabled. The broken optional JWT relay is unset.
The exclusive isolated service is stopped. No account credits were changed.
The existing iMac-built binary was activated without rebuilding.
Binary SHA-256: 602c5c2d6fbbf711c263989852584e62bbc0d927107a220a6659de78de0cab65.
Evidence is in /home/tjninja/.cache/framerize-claude-stalls-20261101/evidence.
The activation receipt is activation-2552520.json. Rollback was not needed.
Long custom adaptive generations still face Framer's upstream time limit.
Next: inspect the next naturally occurring long-request failure with its new
network cause. Do not repeat paid probes or change accounts to evade credits.

# Historical Claude timeout and credit-error repair

The controlled long Claude request completed with HTTP 200 in 561.606 seconds,
2,475 words and 49,914 thinking tokens. Its evidence is preserved. The same test
workspace subsequently returned HTTP 402 on both Claude and free gpt-4.1.
Do not repeat inference until credits on that same workspace are restored.
The user has been asked once. Production still has PID 2340122 and is untouched.

The credit refusal exposed a separate defect: HTTP 402 became HTTP 500 and
paid streaming failures were only comments. The working correction preserves
payment status, gives the actual credit cause and emits typed terminal errors.
It retains the credit floor and never switches accounts. Local credit checks
no longer advise switching to a free model when the workspace has zero credits.
Qualification uses a new remote source and `/evidence/claude-credits-r1`.
All 92 protocol tests pass after recompiling the transferred source. The first
post-transfer run reused the old executable and is excluded. Server r2 passed
473 tests with three failures. The new credit test retained bounded output
buffers; it now uses the existing releasing consumer. The legacy paid-error
test now checks typed failures while preserving every cleanup assertion.
The Unicode deadline case passed alone with unchanged code.
Server r3 passed all 476 tests with two test threads. Session 20227 is complete.
Its six-file manifest and passing qualification are saved in
`/evidence/claude-credits-r1`. No runtime code changed after server r2.
Source `37ec4b1c97a485530268778d4f2c32efafb7cc31` is committed and pushed.
Its release build is running in execution session 96476 through
`build-credit-candidate.py`. Do not duplicate that build.
Next: collect through `collect-credit-package.py`, then start with
`--run-name candidate-credit-r1`. Stop only the old isolated unit before its
successor binds 8792. Old isolated PID is 2429608; live PID is 2340122.
`probe-credit-admission.py` verifies the new binary's credit refusal. At zero
credits the existing guard reads the ledger and prevents upstream inference.

Previously qualified source `3b3ae8231fb76a670b0719fd464c55cde9d14bad`
is published in PR28. All 1,162 affected Rust tests pass by composition.
The first API run found 31 old refund expectations; the corrected API run
passes all 441 ordinary tests and both category-specific tests. Core 155,
protocol 90 and server 474 retain the passing first-run results.
The exact release build and all nine package checks passed in session 51623,
through the existing Docker SSH builder.
Its evidence directory is `/evidence/claude-stalls-r1`.

Public proxy retry-header forwarding is also fixed and its seven tests pass.
That source is `63600d2f` on `ops/public-inference-20261101`, published in PR27.
Neither candidate is live yet. The activation helper updates the proxy only
after the backend has drained, and has four proxy rollback tests plus the
eight backend activation tests. No live mutation has occurred.
The build and all nine package checks passed on 2026-11-02. The archive is
collected and staged in the Claude cache. Binary SHA-256:
`969105d20af12621963865d9bc93b378e388b880a5e99313cd5850727fe13663`.
Candidate PID 2429608 on 8792 passed health and readiness. Production still
has PID 2340122, with unchanged start time and zero restarts.
The first long request timed out at 840 seconds after the test's 1.5 GiB
MemoryHigh caused sustained reclaim. Evidence is preserved. MemoryHigh is now
2.25 GiB, matching production's 90% ratio; the 2.5 GiB hard cap is unchanged.
A new short Claude request passed in 4.169 seconds. The bounded long request
also completed successfully; session 44916 has finished. Do not duplicate it.
Stream, tool, and four OpenAI successes still require restored test credits.
Failed proofs remain separate. No activation occurred.
Isolation explicitly unsets the broken optional JWT relay. Activation does
the same after local-mint verification and checks all acceptance file hashes.
Activation also handles SIGTERM through rollback; all eight rollback tests pass.
Keep each remaining write below 20 lines, per the user's tool-size correction.

Branch `fix/claude-stalls-20261101` prepares `0.5.1-alpha.10`.
Production remains alpha.9 at source `132af226`, PID 2340122.
No live restart or release mutation occurred during this diagnosis.

The isolated browser trace proves HTTP 504 at 60.122 seconds, hidden from
page JavaScript as NetworkError. Long buffered adaptive Claude requests
are the recurring failure. The candidate retains that network cause,
prevents uncertain buffered replay, and streams adaptive requests when
metered access is explicitly enabled. It never requests billing refunds.

The 51 focused Python checks pass. A real browser passed 129 local HTTP
504 observations with request ownership and listener cleanup intact.
Evidence and local helpers are in
`/home/tjninja/.cache/framerize-claude-stalls-20261101`.
Rust qualification and packaging completed in the existing Docker SSH builder.
The local `evidence/package.json` binds the result to the source and hashes.
The first Rust run's failures remain recorded; corrected results are separate.

The candidate on 8792 exclusively owns the non-production profile at
`/home/tjninja/.local/state/framerize-browserless/enrollment-20261023-r1`.
Next: finish the credit-error regressions and successor package. After test
credits recover, use fresh proof filenames for the remaining real checks.
Do not run the old qualification sequence or repeat the completed long request.

# Alpha.9 readiness correction, 2026-11-01

The public endpoint is running at `https://iris-1.javanese-mercat.ts.net:10000/`.
Port 443 is private again. Its separate proxy is recorded in PR27.

Alpha.8 was packaged and passed four isolated real requests. First activation
rolled back safely because an abandoned readiness response falsely closed
admission. Production is alpha.7 again, PID 2294848. No old candidate processes
remain. The test profile is released; the immutable candidate is still staged.

The working change completes only the finished readiness diagnostic. It keeps
the history check owned after caller cancellation. Two regressions cover both
cases. `readiness-before-cached` reproduced the failure. Corrected qualification
runs as `readiness-after-fresh` in `framerize-repair-build-20261101` on
`framerize-imac-ci`. An earlier run used a stale test executable and is excluded.
Use the Docker SSH interface. Do not restart its build or the live service.
The next step is finish the corrected server qualification and build
alpha.9 with `build-readiness-release.py` in the local repair cache.

# Earlier recurring-error repair, 2026-11-01

Current continuation: the final cooldown-cause correction and updated
identity-preservation regressions are undergoing Rust qualification.
The job is `/tmp/verify-final-r4.py`, in Docker container
`framerize-repair-build-20261101` on `framerize-imac-ci`.
Access this restricted builder with `docker -H ssh://framerize-imac-ci`;
ordinary SSH commands are not its supported command interface.
Progress is `/evidence/rust-final-r4.log`; completion is
`/evidence/rust-final-r4.exit`. The source manifest is
`/home/tjninja/.cache/framerize-recurring-errors-20261101/evidence/source-final-r4.json`.
All 40 files match the current worktree. Earlier Rust failures are preserved;
only the unchanged framer-api result, 438 passes, is reusable from r2.
The 34 Python cooldown/recovery checks passed with matching source hashes.

Matched memory r5 passed and exited 0. Both versions completed 512 requests.
Baseline final PSS: 1,437,474,816 bytes, ten pages. Repair: 640,508,928 bytes,
three pages, post-warmup range 638,704,640 to 665,662,464 bytes.
The active stream continued throughout. Evidence is copied to the local cache.
Only the separately tested shutdown error text and whitespace differ from
the exercised memory runtime. `final-source-assessment.json` records this.

Seven activation/rollback tests pass. The activation helper now includes a
real live Responses check inside its rollback boundary. Isolation also has
its own `FRAMERIZE_GATE_STATE`, and its model-list check respects each dialect.
No candidate activation occurred. Live PID remained 2163191, alpha.7.
The next action is collect Rust r4, build the exact committed source with
`build-final-r3.py`, and use the existing isolation and activation helpers in
the local cache. The separate test profile is authenticated and outside the
twelve production accounts. Preserve all safety gates.

## Earlier continuation

Branch: `fix/live-recurring-errors-20261101`. Candidate: `0.5.1-alpha.8`.
The live service still runs the original alpha.7 release. No activation by
this repair has occurred. Its intervening restarts had other unknown owners.
The first isolated build/startup passed. The new active-stream memory fixture
completed 512 requests with three pages retained and 555-677 MB process PSS
after startup. This is local fixture evidence, not a six-account live soak.
The 289-case Python run passed; subsequent cause checks passed 38 tests.
Rust verification is running on the existing separate Linux builder.
The separate saved test account passed authentication with its expected
identity. Its isolated preflight on 8788 stopped cleanly while the final
package builds. The normal history restrictions remain. The UI build passed. Clearing inactive
compiler cache reclaimed 5.85 GB, with binaries and live state preserved.
Next: finish the identical-load memory comparison and Rust checks, build the
exact package, prove real isolated delivery, then activate and verify live.

Final package source: `fc22ddd10d82b4fff96f80eb22b00ceedf3d1ebd`.
Its additional shutdown-cause regression passed with seven related checks.
Rust code is unchanged. The final package job is queued behind those checks
in `framerize-repair-build-20261101` on `framerize-imac-ci`.
The matched baseline completed 512 requests. Candidate run r5 is active in
`framerize-memory-proof-20261101`; earlier setup-cache admission failures remain
recorded. The test's 2 GiB memory limit and admission guard are unchanged.
All local helpers and evidence are under
`/home/tjninja/.cache/framerize-recurring-errors-20261101`.

# Historical admission failure checkpoint

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
