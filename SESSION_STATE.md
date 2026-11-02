# Custom-route-only correction, 2026-11-02

The user requires custom routes only, with no metered fallback. The earlier
561.606-second success used paid streaming and does not qualify this release.
Do not repeat the credit-top-up request or use refunds or other accounts as workarounds.
The old isolated paid candidate is stopped. Production remains untouched at
PID 2340122, alpha.9, with health and readiness returning 200.

The working correction prevents custom routes from selecting paid transport,
including deferred requests and metered-only models. Explicit paid endpoints,
header overrides and the direct agent endpoint require operator permission.
Isolation and activation helpers set FRAMERIZE_ALLOW_METERED=0. Old acceptance,
qualification and paid-probe scripts are disabled pending fresh custom proof.
The iMac builder now has three CPUs and 3.5 GiB RAM; new builds use two jobs.
R1 finished with 440 API passes, one invalid fixture assertion and two ignored
category-specific cases. The assertion expected browser-specific wording from
a generic bridge failure. It is corrected without changing runtime code.
R2 runs in session 93136 through /tmp/verify-custom-r2.py. It reuses those
440 passes, checks the repaired case and runs the 481-case server suite.
Evidence remains in /evidence/custom-route-r1. Do not restart either run.
The Test page now uses the custom API with an explicit model and full local
conversation history. Four web tests and the UI build pass. Two helper tests
confirm stale paid proofs cannot execute or authorize activation.

The prior credit-error package finished and passed its nine package checks.
Its SHA-256 is b74e50bdf505584aedfa6a1e1a31bf73321310af0e14dce4cacbcfc9a4eb20d5.
It is staged privately and is not approved for activation. Next: finish the
custom-route tests, publish and package the exact source, then verify isolation.

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
