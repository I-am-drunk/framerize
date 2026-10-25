# Development status

R5 switched to the qualified candidate, then switched back after its 90-second
startup settling check expired during sequential browser prewarming. The old
binary is running; final rollback verification remains pending. No acceptance
POST ran. See `reference/analysis/r5-rollout-result-20261025.md`.

The normal interruption repair now has a built candidate from qualified source
`93dad965`, executable SHA256 `f70458d364ec624006b5dc968fda49c06ab22aa65673c56fc43b87d7d223aba3`.
Host, API, server and link builds passed and are collected. Its fifteen-file
binding retains the previously qualified normal host and state-preservation
support. The candidate ran during R5 and all fifteen original files still match.
The service now runs the original executable after R5's rollback.
The accepted twenty application cases were not repeated.

The repair retains the next root execution slot ahead of independent signup
and V3 work. The fresh controller now has 30 accepted focused cases: 15 for
readiness/rollback and 15 for shutdown, ingress holds and cleanup. Seven new
shutdown cases verify both paused owners terminate before systemd stop and
that an unreaped dead sidecar cannot allow ingress resume. Test resources were
collected and the original service generation reverified on 25 October.
This supersedes older timed-quiet scheduling descriptions below. The user has
authorized the rollout and checks. Attempts before R5 ended before switching;
their controllers and authentication helpers are collected. An existing idle
close reconciled one uncertain browser without restarting the service. The
later attempt still found active work and could not establish quiescence.
Live interruption and subsequent-stream acceptance remain unverified. See
`reference/analysis/normal-repair-controller-20261025.md`.

Source `191dabe8` adds seven fixed V3 failure labels using the wrapper's existing
structured diagnostics. It preserves broad failures when those diagnostics are
missing or ambiguous and leaves success acceptance unchanged. All 16 JavaScript
and 15 Rust selected cases now pass, with no selected skips. The test build and
both isolated runs are collected. The original wrapper qualification is retained.
No new SDK or native-service acceptance follows from these inert checks. See
`reference/analysis/v3-phase-diagnostic-qualification-20261025.md`.

The updated V3 application now builds, executable SHA256 `7a10ec27`. Two
collected preparation failures exposed missing transitive libraries and bundled
linker tools. Adding their exact bindings allowed the fresh link to pass in
14.2 seconds, with 466,911,232 bytes peak memory and no limit events. All 797
inputs remained unchanged. The worker library, five unchanged libraries and
31 diagnostic passes were reused. No V3 application or SDK was launched.
See `reference/analysis/v3-application-build-20261025.md`.

Separate authority source `52019edc` negotiates ledger versus observation
authority and carries authenticated observation proof through V3 admission
and completion. All 34 selected Rust cases and 21 Python cases now pass.
The returned fixture correction seeds an existing unexpired request before
verification and observation, retaining its whole-history assertion. Only that
case was rerun; the other 54 passes are retained. The isolated run and cleanup
are collected, with unchanged inputs and no resource-limit events. All five
affected libraries were rebuilt before this fixture correction, and the
matching application linked as `6a2f0a93`. The build and test jobs are collected.
This source is not included in the normal repair candidate. No SDK or account
request ran. See `reference/analysis/v3-authority-qualification-20261025.md`.

The matching authority runtime package is now staged and collected. Its
20 runtime files bind application `6a2f0a93`, generated host `f519bb3f` and
support manifest `97d906d3` to the qualified source. All 29 inputs remained
unchanged; the staging unit, cgroup and captured processes are absent.
The application subsequently started successfully in an isolated empty store
with the V3 configuration and served the expected UI assets. That startup
and its processes are collected: exit 0, 31,850,496 bytes peak memory, no swap
or resource-limit events, and unchanged inputs. No SDK operation or upstream
request ran. Current genuine account/device proof remains required for the
bounded live V3 attempt.

Three controller diagnostic/privacy cases also pass. The next prepared rollout
controller preserves fixed failure codes and bounded source locations without
recording arbitrary exception text. It is bound to the unchanged repair package
and was launched once. Its 25 October 03:08 UTC collection records
`positive_quiet_window_not_reached` before capture, switching or either live
check. Both helpers are collected, with the original service unchanged.
The bound controller still prohibited request cancellation despite the user's
later explicit permission to interrupt streams. A fresh invocation must support
that permission without bypassing the state-transfer cleanup contract.

The separate extracted V3 wrapper checks passed on 25 October: nine selected
cases and 35 leaf scenarios, with no skips. They confirm cache, strict-null
fallback and timeout behavior under inert loaders and virtual clocks. Caller
timeout can leave initialization reusable; guarded phase failure disables V3.
Returning a result or timing out does not cancel the underlying operation.
The isolated job is collected and the shared slot is free. This is wrapper
control-flow evidence only; no SDK, token or live service was exercised.
See `reference/analysis/v3-wrapper-control-qualification-20261025.md`.

The explicit-account retry correction now passes all twenty selected cases:
six corrected cases passed on `93dad96`, and the fourteen unchanged passes
are retained. The fixture now polls the existing shared cleanup observer
while preserving its unread response and original assertions. Production
code and timeouts did not change. Both jobs are collected under the original
guards. This qualifies the source change; deployment and real interruption
acceptance remain separate and unverified.

The initial combined explicit-account retry and invalid-header change ran its
exact 20-case selection on composed source `1676f57`. Both bounded test builds
pass. All twelve existing controls and both invalid-header cases pass; the
six new retry cases time out at their shared producer-completion wait.
That qualification failed. The exact failure was returned to Server for a
bounded fixture correction; no production cause or rollout acceptance is
claimed. This ran independently of signup and Castle research, with no service
action. See `reference/analysis/explicit-account-qualification-20261025.md`.

A public Framer login diagnostic reached the actual sign-in API with an email
and nonempty verification token, then received HTTP 403, “Access denied by
policy”. The page reduces that error to “Verification failed”. No missing
form step or specific browser defect was established; the policy's reason
is undisclosed. Dedicated signup and native-token acceptance remain pending.
See `reference/analysis/framer-signup-policy-refusal-20261025.md`.

The unchanged account-management web UI at source `6ed2d5a` now builds.
All five bounded stages passed in 11.1 seconds, with 272,695,296 bytes peak
memory, no swap or limit events, and all 1,941 input hashes unchanged.
The resulting assets are staged only. No daemon was started or login sent
by that build, and its scope and all captured processes are collected.

Fresh-account admission now accepts the same host's verified session and
joined SDK observation while retaining its original rate ledger. Fifteen
focused synthetic tests pass, including two consecutive native mint permits,
real cooldown/capacity refusals, caller identity and cleanup checks, and ledger
replacement during spacing. An existing enrollment fixture needed its missing
`asyncio` binding restored. This is source qualification only. Dedicated login,
runtime packaging and live native-token inference remain unverified. See
`reference/analysis/fresh-account-native-admission-20261025.md`.

The fresh same-device transport comparison now passes: browser and native HTTP
both returned complete, nonempty HTTP 200 responses using browser-generated
tokens. Cleanup was confirmed and updated rate history preserved before the
isolated service was retired. The normal 8787 service was unchanged. This used
the resident coordinator without its native SDK worker, not the ordinary V2
package. Native token generation remains the next unverified boundary. See
`reference/analysis/browser-token-native-http-20261025.md`.

The authorized rollout switched to the candidate and obtained its restoration
receipt, then failed startup quiescence before either stream test. Reverse
startup was attempted, but receipt verification failed. The later service log
records a restart blocked by the remaining activation fence. The old binary
is now running again after a separate recovery whose actor is not established.

The user ended blanket cooldown scheduling. A direct request on the selected
account returned a complete `OK` stream with HTTP 200 on the current service.
Its account prefix matched uniquely and the process generation stayed fixed.
This is a browser baseline, not a completed browser/native comparison. The
next comparison must use the owned admission path without the old runner's
additional 600-second reservation. No new rollout is scheduled.

The first scheduled rollout was refused before authentication handoff or
service mutation. At collection the original service was idle and exact,
but an account cooldown still had 273 seconds remaining. A model request
during a forced tool-wake interval had triggered another refusal. The next
attempt needs uninterrupted agent quiet, with the same qualified controller
and a fresh coordination window. Both live acceptance checks remain unspent.

Continuing 8787 traffic was traced to the shared Codex backend that also runs
this investigation. Nine inert checks now qualify a separate scheduler for
the unchanged rollout controller, allowing model traffic to stop during the
transition. The real rollout is still unrun. See
`reference/analysis/normal-rollout-self-dependency-20261024.md`.

Normal-rollout preflight exposed malformed event-log records. Six focused
reader cases now pass; plan `ad5f21ad` preserves the approved rollout and adds
only the corrected reader binding. The old plan and authentication evidence
are retained. Actual active work and a cooldown, not the parser, now hold the
rollout. No stop/start or stream test has run. See
`reference/analysis/normal-rollout-quiescence-20261024.md`.

The user delegated the first ordinary-V2 identity choice. The selected path is
a fresh verified sign-in under the current contract, followed by the existing
observed SDK call. Saved cookies alone remain insufficient. No sign-in, mint
or native inference has been performed by this selection.
An empty host still needs a valid rate-state admission path: normal login
creates history which the separate initial-enrollment endpoint refuses to
overwrite. This independent source prerequisite is recorded in
`reference/analysis/ordinary-v2-first-login-rate-boundary-20261024.md`.

The generated native host now passes its actual entrypoint and command loop
with empty isolated state. Native-open without restored ownership is refused,
status remains unchanged, and EOF exits cleanly without persistent files.
The corrected bytecode-isolated run peaks at 46,804,992 bytes under 128 MiB,
with zero swap and no limit events. All 732 inputs match; both jobs are
collected. The initial functional pass and its unpinned-bytecode limitation
are preserved. This covers explicit system Python startup, not the installed
browser environment, real identity transfer or minting. See
`reference/analysis/ordinary-v2-host-startup-20261024.md`.

The packaged native daemon now passes an isolated startup check with explicit
V2 configuration, a fresh empty database and no eligible accounts. Health,
empty-account listing, offline input rejection and `/api/hello` pass. Peak
memory was 28,422,144 bytes under 128 MiB, with zero swap and no limit events.
All 1,935 inputs remain exact. The first checker used the wrong hello URL;
that collected failure is preserved. The corrected run is collected as well.
No SDK, browser or live account was used. The next real native run requires
a fresh verified sign-in under the current contract, or a separately selected
verification path for saved sessions. See
`reference/analysis/ordinary-v2-daemon-startup-20261024.md`.

The assembled native worker now passes one isolated startup rejection check
through the exact qualified Rust process owner. Its unchanged Node executable,
worker entry and configuration reject invalid input before an SDK operation.
The original worker joins and exits. Runtime peak memory was 25,579,520 bytes
under 128 MiB, with zero swap and no memory or task limit events. All 1,917
bound inputs match before and after, including the private library snapshot
mounted read-only at the required location. Compile and runtime jobs are
collected. This is startup and rejection evidence, not a valid mint or inference.
The normal rollout remains authorized and unexecuted while the previously
requested terminal authentication is pending. A fresh hold and positive
endpoint quiescence remain required. See
`reference/analysis/ordinary-v2-worker-startup-20261024.md`.

The normal-rollout controller now fences startup while it changes ownership
of saved host limits. A failed start can reuse a transfer only when two checks
prove it is still unused, the original generation is gone, the manager has no
pending job, and the entire service scope is empty. Any restore claim or
ambiguous publication refuses reuse. The original snapshot and request are
preserved before the rollback target changes. Failure cleanup closes the fence.

Fifty-eight focused checks pass across five isolated jobs, retaining the
128 MiB memory limit, no swap and private network/filesystem isolation. These
cover the predicate, actual evidence adapter, state publication and controller
failure paths, plus the terminal authentication adapter. All jobs are collected.
The exact 15-file release and original service generation were reverified.
Plan `9f3c55fc` is statically ready; it has not run. Administrator authentication
for the read-only memory descriptor is not cached. A bound terminal helper is
prepared, and a fresh client hold plus positive quiescence are still required.
No service stop/start, live stream or native activation occurred in this work.
See `reference/analysis/normal-rollout-startup-recovery-20261024.md`.

The normal-stream acceptance client now checks actual generated text before
disconnect and correlates the response's opaque operation ID with the original
producer join and both successful releases. Ten new synthetic checks and one
local socket check pass under a 128 MiB memory limit, no swap, isolated
networking and no private runtime mounts. The original SSE parser is unchanged.
Both jobs are collected; the captured launcher/client processes and groups are
absent. This does not claim exhaustive transient descendant capture.

The normal-host package now passes five inert class/support compatibility
cases. Its 15 files retain binary `314c3190`, generated host `f94e2d48`, and
the existing cooldown support. The compatibility run peaked at 67,944,448
bytes with zero swap and memory/task events. It skips the generated entrypoint
and does not establish a real state transfer. Seventeen new inert controller
checks pass for override cleanup, retained limits, recovery selection and
fresh-capture rollback ordering. Result `23a7b56f` and its exact source snapshot
are retained. The launcher and full controller remain unqualified preparation.
The live pair remains unspent and no live service changed.

The 15-file package is now copied and independently verified in a separate,
inactive release. Five additional quiet-window cases pass on the integrated
callback, six launcher/runtime-binding cases pass, and two forward-flow
cases pass with external effects stubbed. The forward flow verifies state
before issuing the stream pair and refuses a stop when the capture window
expires. All four new jobs are collected. Failed-start recovery before a
replacement host exists remains an explicit preparation gap; the controller
plan remains unready. No real transfer, restart or live stream has occurred.

Updated 2026-10-24. The normal service remains browser-based. Publishing these
branches does not change the running service.

The separate launch-route fixture is integrated as `46a2352`. Its four cases
pass (`5ddaf7aa`) with a 36,515,840-byte peak under the retained 128 MiB cap,
zero swap and no memory-limit events. The launcher, direct unit client and
recorded stage have exited; the unit and cgroup are gone and the shared lock
is free. This does not claim exhaustive transient descendant capture. A
separate receipt parser accepts the fixture's interleaved logging; the tests
were not repeated. The prior 79 V2 results and production binary are unchanged.

The original-join correction is integrated as `e7da66e`. All 26 selected
cases pass in disjoint 6, 6 and 14 case projections with unchanged limits.
The original combined compile reached its 768 MiB limit; that result is
preserved. Split compile peaks were 457,482,240, 480,702,464 and 604,119,040
bytes, with zero swap and no memory-limit events. The server builds
(`a15b20ad`) and executable links (`9fc32074`, binary `314c3190`). All eight
successful jobs and the initial failed compile are collected.
The new normal rollout proposal binds that executable, seven source-exact
Python modules, explicit ownership startup, joined acceptance and rollback.
Its eight-file package passes static checks and has not run. The earlier
`95f29867` package is preserved. The user authorized the reviewed rollout.
Final preflight verified the original service generation and 7,795 external
runtime files, then copied and verified the inactive release. No drop-in,
stop/start or stream test occurred. The selected raw host lacks the installed
host's identity-bound device-limit restoration hook. Account cooldown expiry
alone does not establish expiry of the separate host device ledger. A bounded
normal-host transfer or positive expiry proof remains required before stop.
The linear Python pipe patch is preserved through lazy `browser_runtime()`
initialization. Existing source/test receipts remain valid.
See `reference/analysis/normal-browser-joined-qualification-20261024.md`.

The separate native package retains binary `6012de1c`. Its exact reviewed
staging invocation passed once: 20 runtime files and 16 source inputs match,
including the generated host, 11 support modules and six adjacent imports.
Peak memory was 61,972,480 bytes under 128 MiB, with zero swap/events. Its
terminal and captured processes are collected. Neither application package
has started a host or worker, and native acceptance remains unverified.
The native package subsequently passed its isolated CLI help and all 18
Python definition imports (`77161c4f`), with a 30,105,600-byte peak and zero
swap or limit events. All recorded process identities are absent and the
unit is collected; the external snapshot missed transient descendants of
the 1.73-second run. The 1,234-entry external library snapshot is copied and
verified but has not been mounted or used by a worker.

The ordinary-V2 integration branch now includes browserless `95f2986`,
including the qualified legacy and typed-capacity corrections. Only the two
status documents conflicted; both sets of results are retained below. This
merge does not repeat their tests or extend those results to the SDK transfer.
Final transfer input `838f7dbb` is now integrated. All 22 incoming paths match
their owner bindings, while all 37 core files match the completed intermediate
build. The newer typed-capacity and router work is preserved. All 31 Rust
cases pass (`292133be`), as do all 48 Python/Node cases (`4f1b5d34`). Test
compilation passes (`1884a4c5`), all four dependent libraries rebuild, and the
production executable links (`cbf5e604`, binary `6012de1c`). The explicit
prepared host remains required for runtime; its package is now staged as
recorded above, without application execution.
The intermediate core compiles (`6459710f`) with a 167,043,072-byte peak under
the existing 512 MiB limit, zero swap and no resource-limit events. Its launcher
is collected, captured child absent and group removed. Exact core-source
equality with final integration `8b5a43c` permits reuse. All seven subsequent
Rust build/test jobs used the shared lock and original limits; captured PIDs
are absent and units/groups removed. Python terminal, recorded stage PID and
group cleanup are verified, but its separate external full-PID snapshot was
missed before completion. That limitation is retained, with no repeat run.
See `reference/analysis/ordinary-v2-transfer-qualification-20261024.md`.

Branch `integrate/ordinary-v2-identity-20261024` assembles the 15 executable
paths from ordinary-V2 candidate `fe500a0` onto qualified `0b1107c`.
It retains the current server corrections and browser-only ownership startup.
The ten selected startup cases pass on assembly `3d52cde` (`5b0dd573`),
covering explicit V2/V3 selection, default V3, incompatible modes and current
browser ownership startup. Test compilation passes (`d08b2037`), and the
production executable links against unchanged qualified libraries (`ac14bb65`).
Measured peaks were 602,656,768 bytes for test compilation, 14,852,096 for tests
and 523,440,128 for linking, with zero swap. Input bindings, selection and
cleanup were verified. The executable was not launched. Prior shared-worker
and post-handoff results belong to the separate candidate.
The owners retain fixed base `3d52cde` while this branch combines their work
with the qualified server corrections. The new checked transfer is tied to
account, session, incarnation and original grant; HTTP-device fallback is
removed from the V2 seed path. Synthetic qualification passes. No live activation is
authorized by this integration.
The selected source contract adds one read-only validation of retained SDK
anchors immediately before the existing browser close, within its remaining
deadline. Failed validation refuses transfer without preventing close/join.
Successful validation plus joined close is a point-in-time guarantee, not
proof of continuous JavaScript equality or quiescence during close. This
producer/consumer contract still requires native runtime acceptance.

The six-path typed-capacity input `2f165027` is integrated over `41484eb`.
Seven core cases pass (`4e10d73d`, 44 source-enumerated scenarios), and six
actual-router cases pass (`fb86a3c6`, 40 scenarios). A current cooldown or
non-Ready lifecycle produces a typed capacity refusal; `/api/run` maps only
those variants to its existing HTTP 503 responses. Full cooldown seconds
remain in the response while Retry-After stays bounded to 1..30 seconds.
Missing or changed scope, foreign sessions and unrelated upstream errors
retain their existing category, even when error text mentions capacity.
Ready controls still succeed. Test compilation passes (`9cf35e3f` core,
`98f246d7` server). Core, host, API, store and server libraries rebuild, and
the final executable links (`2d4b2617`, peak 449,380,352 bytes). All ten jobs
used shared execution locking and capped isolated runners; their source and
output bindings, zero swap and resource-limit events, terminal states and
absent child processes were verified. This does not establish service rollout
or native acceptance. The ordinary V2 producer/consumer package remains the
next separate integration input.

The six-path legacy completion input `397c9ed5` is integrated over `0b1107c`.
All 33 selected tests pass (`a2512ffa`, peak 16,248,832 bytes), covering 283
source-enumerated scenarios. Compilation passes (`2f402c7c`, peak 511,803,392
bytes). Paid aggregate output completes after collection; raw output completes
after EOF and producer join. Early disposal remains unconfirmed. Management
and import operations retain cleanup evidence separately from original
results, and pending import children keep their own completion verdict.
The first paid permit wait now notices disconnect, preserving any prior
uncertainty while allowing an independent account to proceed. Fifteen new
cases and 18 affected controls pass with exact selection and source bindings,
zero swap and resource-limit events. Shared execution locking was explicit.
Production server compilation and executable linking pass (`e3613b9e`,
`60a445ba`), with peaks of 385,220,608 and 444,715,008 bytes and zero swap.
All four launchers were collected; process groups were removed and captured
child PIDs were absent. These fixtures do not prove service rollout or native
acceptance. Ordinary V2 remains a separate assembly while its host and Rust
owners implement the identity transfer.

The two-path native HTTP input `bb2e807b` is integrated over the qualified
local/static server. All five selected tests pass (`84dbed12`, peak 12,423,168
bytes), covering 62 scenarios. Test compilation passes (`82d3808f`, peak
392,339,456 bytes). Local refusals and completed nonstream output release HTTP
ownership while admitted native cleanup keeps its independent owner. A timed
out acquisition remains unconfirmed. The cases cover consumed and discarded
bodies, delayed cleanup and waiter disconnect; existing stream observers
remain unchanged. Exact inputs, selection, zero swap/limit events and removed
groups were independently verified. The first runner attempt refused a unit
name mismatch before compilation; the corrected preparation preserves it.
The server build and executable link pass (`8ea7d239`, `c839fcae`); link
peak was 531,582,976 bytes with zero swap. Their inputs and cleanup were verified.
This does not supply the ordinary-V2 observation transfer or prove native
acceptance. No executable launch or service change occurred.

The six-path local/static input `3ab7c169` is integrated. All eight selected
tests pass (`f7040c99`, peak 13,422,592 bytes), covering 128 scenarios.
Compilation passes (`3124668c`, peak 428,265,472 bytes). Completed local
responses, extraction failures and explicit pre-work refusals release request
ownership. Static responses retain it until their inner body is consumed or
disposed. Full, unread and partial GET plus HEAD cases pass, as do the existing
paid dispatch and interrupted-call controls. Exact inputs, selection, zero
swap/limit events and removed groups were independently verified. These cases
do not simulate an in-flight file-read failure or prove OS descriptor closure.
The server rebuild and executable link pass (`fe73c9fc`, `62f41c47`); link
peak was 461,238,272 bytes with zero swap. No service was changed.

The six-path `/api/run` input `61d2e64b` is integrated. All 17 selected tests
pass (`fa69eea2`, peak 12,496,896 bytes), covering 41 scenarios. Compilation
passes (`755ecbf5`, peak 439,271,424 bytes). The selection covers completed
output, disabled accounts, malformed requests, acquisition failures and
unknown inference cleanup with consumed and discarded response bodies, plus
changed explicit-account and direct controls. Original errors and the
unmanaged-host barrier remain. Exact inputs, selection, zero swap/limit events
and removed process groups were independently verified. The server rebuild
and executable link pass (`bea01ccd`, link peak 454,189,056 bytes, zero swap).
No prior suite was repeated by parentage. Legacy/admin
completion and typed capacity mapping remain separate source inputs; no
normal-service activation or successful native request is claimed.

The two-file browser startup input `121d770a` is integrated. All five selected
tests pass (`b36564d4`, peak 14,446,592 bytes); test compilation passes
(`df140c78`, peak 353,198,080 bytes). The explicit option enables ownership
without enabling native routing. Tests preserve default behavior, reject
mixed modes, retain the host retirement barrier and serve the normal hello
routes. Exact bindings, selection, zero swap/limit events and process cleanup
were verified. The executable links against unchanged qualified libraries
(`b9114054`, peak 452,710,400 bytes, zero swap). Selected support closure, remaining shared-router corrections and
normal-port interrupted/subsequent streams remain rollout prerequisites.
No protected-service change or executable launch was performed.

The two-path pipeline admission input `b7aed0f5` is integrated. All seven
selected tests pass (`aae27a4c`, peak 11,575,296 bytes), covering 35 scenarios.
Test compilation passes (`59eeca16`, peak 231,718,912 bytes). A cooldown or
non-Ready state recorded during the first call prevents the 402 retry; replacing
the account with an eligible Ready record still permits one successful retry.
Four affected controls preserve result and cooldown handling. Independent
verification accepted exact inputs, case selection, zero swap/limit events and
removed process groups. The core and its host, API, store and server dependents
also build successfully. The integrated executable links (`303d97e2`, peak
461,455,360 bytes with zero swap); its inputs and cleanup were verified.
There is no new initial-workspace-await discriminator. The update lock ends
before inference, and `/api/run` still maps these refusals to HTTP 500 pending
its separate endpoint correction. No executable launch or service change.
The current protected command would still leave request ownership disabled.
The startup option is qualified above. Its actual startup/support binding and
the remaining shared-router completion groups precede any reviewed rollout.

The three-path local-refusal input `345a9271` is integrated. All three selected
tests pass (`33a2d86b`, peak 12,238,848 bytes), covering 16 scenarios. Test
compilation passes (`0b872675`, peak 429,654,016 bytes). Pure validation errors
retain their original response and release local ownership; an account becoming
unavailable during its first permit wait also releases ownership after an
unread refusal. An independent account then completes successfully. Exact
pins, selected cases, zero swap/limit events and removed groups were verified.
The prior 13-case batch was not rerun. The two metered-policy branches and
earlier pre-session rechecks have no new direct fixture discriminator.

The changed server library and integrated executable also build successfully
(`5d2cc33f`, `9e42450b`), reusing the unchanged qualified dependencies. Link
peak was 448,905,216 bytes with zero swap. Independent verification accepted
the source bindings, limits and process cleanup. No executable was launched
or deployed; normal-service recovery and native acceptance remain unverified.

The six-path pre-inference input `330bedb8` is integrated. All 13 selected
tests pass (`73571c33`, peak 12,488,704 bytes), covering 65 parameterized
scenarios. Compilation passes (`3f0eb215`, peak 431,255,552 bytes). The batch
covers initial-acquisition cleanup proof, normal JSON extraction failures,
disconnect during initial and later nonstream permit waits, and the existing
admitted-call cancellation control. Original errors, paid forwarding, prior
cleanup uncertainty and cooldown are retained. Exact pins, selected cases,
zero swap/limit events and removed groups were independently verified.
Earlier source-matched selections were not rerun. Equal account snapshots
do not establish zero identical-value writes. Pure-validation refusals and
the initial post-permit ServingCapacity case are covered separately above.

The changed server library and integrated executable also build successfully
(`eb206c93`, `1867120f`), reusing the unchanged qualified dependencies. Link
peak was 457,428,992 bytes with zero swap. Source bindings and process cleanup
were independently verified. The executable was not run or deployed; the
protected browser service, normal-endpoint recovery and native acceptance
remain separate from this qualification.

The three-path streaming follow-up `46d307d8` is integrated over the qualified
account-policy input. All five selected cases pass (`d26026df`, peak 12,603,392
bytes), covering ten scenarios across both normal routes: later success or
plain error cannot erase earlier unknown cleanup, disconnect joins the producer,
and an initial known cooldown refusal leaves subsequent admission open.
Test compilation passes (`01011882`, peak 689,569,792 bytes). The prior 47-case
selection was not rerun. All selected names, pinned inputs, zero swap/limit
events and absent process groups were independently verified.

Only the changed server library and executable were rebuilt. Both pass
(`5cf58557` and `b7038622`), reusing source-matched, previously qualified core,
host, API and store libraries. Link peak was 590,610,432 bytes with zero swap.
No new executable was run or deployed. The subsequent account-repick correction
is recorded below. Native acceptance and
normal-service streaming acceptance are still unverified.

The four-path repick-acquisition input `a34bd3cd` is integrated. Its three new
router cases pass across 24 scenarios, and both directly affected acquisition
controls pass (`59abbf9d`, peak 12,500,992 bytes). Compilation passes (`326be3ce`,
peak 408,113,152 bytes). The cases preserve the original client error and the
single inference attempt while checking cleanup, resident capacity and next
admission after consumed or discarded responses. Unconfirmed close remains
fenced; a confirmed refusal before close leaves admission open. Both phases
independently verified source/input pins, zero swap/limit events and cleanup.
No prior 47/24/8 or streaming-five selection was repeated. Initial acquisition
is covered by the combined pre-inference qualification recorded above.

The changed server library and integrated executable also compile
(`3a3987db`, `fb450b18`), reusing the unchanged qualified dependency libraries.
Link peak was 456,814,592 bytes, with unchanged pins, zero swap/limit events
and removed groups. The executable has not been run or deployed. The existing
browser service remains protected; isolated results cannot establish its
normal-endpoint interruption and subsequent-stream acceptance.

The account-policy candidate `43fca821` is integrated, retaining the previously
qualified client corrections and newer documentation. All 47 selected cases
pass: 20 rotation/cooldown cases (`482719c7`), 18 explicit-admission and request-
completion cases (`d7e9cef7`), and nine project-collection cases (`e8a634ac`).
The three test compilations pass (`b618cd9d`, `3224ac05`, `53c3470d`). Every
phase independently verified its inputs, zero swap/limit events and removed
process groups. No isolation, shared-worker or post-handoff tests were rerun.

Explicit requests now require the selected account to remain Ready with a
complete scope. Rotation publishes a matching team/project pair only after
resolving an existing project, and preserves concurrent health and cooldown
changes. Nonstream Messages cleanup survives disconnect without retrying or
repicking an account. Contradictory project-team metadata is rejected; absent
metadata still relies on the requested team route. These are synthetic fixture
results, not deployment or successful native Framer inference.

The updated core, host, API, store and server libraries also compile together.
The integrated executable links successfully (`5601979f`, peak 523,464,704
bytes), with unchanged input pins, zero swap/limit events and verified cleanup.
It has not been executed or deployed.

The response-excerpt correction from `c2aa393` is integrated in source.
All five sites now truncate at a valid UTF-8 boundary inside the existing
500-byte or 120-byte limit. The refusal policy and fixed sentinel are unchanged.
API test compilation passes (`9cfc2add`, peak 292,302,848 bytes), as do all eleven
`classify::tests` cases (`338fcb1d`, peak 11,927,552 bytes). The test covers every
interior cut of two-, three- and four-byte characters at the 500-byte boundary.
Both phases used pinned input, no network and zero swap, with verified cleanup.
The disk prerequisite was recovered by removing six explicitly released,
clean/published source checkouts; branches and frozen evidence were retained.
No live request, service deployment or change to frozen R16 accompanies this
source integration.

The two-file HTTP isolation fixture from `90cc001` is integrated and
qualified. API test compilation passes (`c0dcaefc`, peak 322,371,584 bytes),
and all eight send cases pass (`bb73da8d`, peak 15,646,720 bytes). The new case
holds two distinct account/session requests, verifies independent credentials
on the wire, cancels/joins A while B retains admission, then completes B once.
Crossed account, session and token-owner inputs cannot reach either server.
Both phases had zero swap/limit events, unchanged inputs and removed groups.
This uses synthetic loopback traffic only; production code and R16 are unchanged.

The extension from `723c940` is also integrated and qualified. Its same eight
cases pass with the concurrent-account case covering ordinary and resident
headers sequentially. Each server checks its own exact cookie, bearer and
token expectations before A is cancelled and B completes. Compilation passes
(`3e3091de`, peak 704,241,664 bytes), and all eight tests pass (`cc058dbf`, peak
15,953,920 bytes), with unchanged pins, zero swap/limit events and absent groups.
Both fixture files are bound together; the earlier `90cc001` result is retained
for its original scope. No production change or live request accompanies this.

The separate ordinary-V2 candidate `fe500a0` now passes both shared-worker
account-isolation cases. Compilation passes (`43a69a78`, peak 362,864,640 bytes),
and the two-case run passes (`4b6d6e37`, peak 25,038,848 bytes). The same worker
serves distinct account/SDK seeds: A's child joins while host acknowledgment
remains pending, B completes, and cancellation or lost acknowledgment affects
only A. B can be admitted again. Both phases verified unchanged inputs, zero
swap/limit events and removed groups. The frozen candidate includes the
ordinary-V2 source stack, which is not promoted into this runtime branch by
these tests. This is inert fixture qualification, not an SDK or live request.

The same `fe500a0` candidate also passes the three post-handoff lifecycle
cases: wait for child exit and host acknowledgment, discard a valid token
after failed exit, and retain cleanup through cancellation. Compilation passes
(`e77ffad1`, peak 395,350,016 bytes), and all three cases pass (`66dca8f8`, peak
24,629,248 bytes). Both phases verified unchanged inputs, zero swap/limit
events and absent groups. These use synthetic sessions and fake workers;
ordinary-V2 serving and actual-native acceptance remain unqualified.

The eight-path client-correctness candidate `29781d33` is integrated in source.
It preserves current account state while recording risk-refusal deadlines on
both inference attempts, marks completed local read handlers before HEAD body
removal, handles one initial response BOM, and keeps server error excerpts on
UTF-8 boundaries. All nine inert cooldown cases pass (`1ab8cfde`, peak
11,935,744 bytes), with test compilation passing (`75bb543e`, peak 441,163,776
bytes) and core library compilation passing (`63abdad1`, peak 176,738,304 bytes).
The exact core in `57f3cd07` is byte-identical to this candidate. Pins are
unchanged, with zero swap/limit events and absent groups.

The remaining fifteen selected cases now pass, completing all 24 for this
candidate. Four server cases pass (`e18dadf9`, peak 12,234,752 bytes), covering
completed read-only HEAD requests, UTF-8 diagnostics and unknown-cleanup
fencing. Eleven response-output cases pass (`df24c87d`, peak 11,771,904 bytes),
with test compilation passing (`21857278`, peak 287,657,984 bytes). They cover
one initial BOM, invalid UTF-8, misplaced or doubled BOMs, and the distinction
between completed model text and incomplete, error or tool-only responses.

Host, API, store and server libraries were rebuilt from the integrated source;
the production executable links successfully (`6191637a`, peak 534,614,016
bytes). Independent verification confirms unchanged inputs, zero swap/limit
events and absent groups. The prior 20 unchanged server controls and eight
native HTTP isolation cases remain valid and were not rerun. No new executable
was launched or deployed. Actual service recovery and native acceptance remain
unverified.

The outstanding goal is a fresh, valid browserless mint followed by accepted
native Framer inference, with matched identity and egress, account isolation,
cooldowns, joined cleanup, streaming, latency and measured memory use preserved.
Completed source tasks and dependencies on another owner do not complete or
block that shared outcome. Goal identity and accumulated accounting are retained.

The recovery owner reports the fixed hook denial in all six live sessions
(`303f00e`). This runtime session also verified its original active goal and
nondecreasing accounting against the preload. The prior quiet window is retired.
Transport reports an explicitly user-created active goal on its replacement
thread. Its older detached goal remains preserved. The old reattachment plan
is retired and must not overwrite the new goal. T3 source qualification stays
separate from the Framer runtime work.

R16 completed with normal browser HTTP 200, browser-baseline HTTP 200 and
native HTTP 403 with the complete Castle security envelope. Cleanup was
confirmed, the isolated host remains fenced, and the protected service was
unchanged. Result `16bce73b` binds the intended `fe48a036` executable and
unchanged realm configuration `514c442b`; all 2,013 activation inputs matched
afterward. Peak group memory was 1,545,809,920 bytes, with zero swap or memory
limit events. Actual cooldown history and the isolated owner are retained.

The new observer captured the actual token structure for both phases of R16:
baseline 105 fields/807 bytes and native 90 fields/567 bytes, paired by lease.
The qualified comparator reports 30 structural differences. This closes the
missing native-observation gap; it does not explain the refusal or establish
native acceptance. No token or decoded field payload was saved. No live retry
is queued while these new structural facts are assessed.

R15 previously completed with normal browser HTTP 200, browser-baseline HTTP 200 and
explicit window-realm native HTTP 403. The native response was complete JSON
with Framer's Castle security envelope. Cleanup was confirmed, the isolated
host remains fenced, and the protected service was unchanged. The live command,
binary `345cf340`, realm package `514c442b` and all 1,441 activation inputs
matched. Result `8580c020` is preserved. Peak group memory was 1,584,971,776
bytes, with zero swap and memory-limit events. No repeat request is queued.

The R15 browser baseline projected to 105 fields/817 bytes. Five entries differ
from R14's accepted baseline: 0/12, 7/23, 7/25, 7/26 and 9/9. Comparing R15
with the prior synthetic realm observation gives 30 differences. Neither
comparison contains R15's native token structure or establishes refusal cause.
The bounded native-token projection is now integrated and qualified offline.
All 40 Rust cases pass (19 decoder, three observer and 18 existing resident/
cancellation cases), receipt `4849df69`, peak 14,753,792 bytes. Test compilation
`5778defc` and production linking `4105dbb7` pass against the retained runtime
library graph, with peaks of 368,992,256 and 452,526,080 bytes. This does not
rebuild or activate all newer API changes. The observer borrows the original
token after joined mint and same-device confirmation, before unchanged
authorization and deadline checks.

The synthetic Rust/Python comparison passes all 231 inputs, including 115
accepted structures (`b4d75a8b`, peak 57,516,032 bytes). Strict record pairing
passes all 15 cases (`201bfcaf`, peak 22,142,976 bytes). All groups exited with
zero swap or limit events; inputs were verified unchanged. No SDK, private
token or live request was used. These fixtures preceded the R16 capture above.
The shared event writer now buffers JSON plus newline and makes one append
attempt, refusing short writes without a suffix retry. Core compilation passed
(`44393cbd`, peak 350,724,096 bytes), as did fixture compilation (`84aae788`,
127,766,528 bytes) and all five tests (`21393fe8`, 12,193,792 bytes). They cover
large escaped records, short writes, write/flush errors and independent append
handles. All phases had zero swap/limit events and confirmed group cleanup.
This does not rebuild the retained executable or change a running service.
The source finding does not establish a corrupt R15 line or explain its 403.

The comparator now raises its fixed failure after leaving the exception handler,
so the caught error is not retained as its context. All 17 cases pass (`a489c417`,
peak 22,474,752 bytes), including direct context/cause checks. No swap or limit
events occurred; inputs were unchanged and the test group exited. This makes
no guarantee about traceback-local erasure. The future collector now rejects
duplicate JSON keys before envelope checks and pairing. Its shared decoder
passes six synthetic cases (`ed4313ae`, peak 17,043,456 bytes), including nested
and escaped duplicates, fixed errors and successful pairing. Inputs matched;
no swap or limit events occurred and the test group exited. No private log
was read. The qualified writer is now linked into the staged capture binary;
R16 freezes the strict decoder with the collector and its helper dependencies.
Accepted native inference remains unverified.

The report projection also rejects non-plain entry keys after validation and
raises fixed failures only after clearing mutable input and leaving the caught
handler. All 15 combined cases pass (`014e8aaf`, peak 17,879,040 bytes), including
disguised string keys, genuine parser output and control-flow exceptions. Inputs
were unchanged, swap and limit events were zero, and the process group exited.
Shared validators remain unchanged. This is synthetic qualification only.

The capture executable is now rebuilt with the qualified writer and current
host, API, store and server sources. All five compilation/link phases pass;
the final link receipt is `3874154a`, binary SHA-256 `fe48a036`, with a
608,927,744-byte peak. Every phase had zero swap/limit events and joined cleanup.
The API build includes the qualified bearer-deadline and response-head changes;
the final executable includes the native structure observer. This is a staged
binary, not a deployment or new inference.

R16 is prepared with 2,013 pinned inputs. Its daemon command and expected image
hash select `fe48a036`; activation and collection both check the captured image
against that intended binary. The collector, JSON decoder, pair helper and
projection module are frozen together. Package binding is `4c8e143f`, diagnostic
binding `1ddef66c`. Six synthetic full-collector cases pass (`e26b541b`), covering
day selection, lease pairing, unavailable native data, duplicate keys, detached
timestamp errors and ambiguous records. Peak memory was 17,993,728 bytes, with
zero swap or limit events, unchanged inputs and the test group removed. Prior
disk preflight refusals ran no cases or requests; disposable caches were reclaimed
without changing the reserve. The subsequent R16 outcome is recorded above.

The request-path correction is integrated as `216350e`. Its exact 19-case
qualification is reused from receipt `65937832`: all pass, 17,039,360-byte peak,
zero swap/limit events and process group absent. The preflight now checks the
actual request argument, so an unused correct assignment cannot hide a stale
literal. R15's preparer already checked that consumed argument separately.

## Qualified milestones and retained history

Three source milestones are qualified for future packages. Dedicated-worker
cleanup now retains pending termination promises after worker exit and preserves
termination failures. All 17 inert cases and both real-thread cases pass, with
a 73,641,984-byte peak and no swap or limit events. The structural comparator
passes all 16 cases (17,321,984-byte peak). It distinguishes missing observations
from empty structure and reports only field presence, type and encoded length.
The pinned-source interpretation note maps four group-7 differences, including
the limits of Boolean tags, transformed-string lengths and timing-field omission.

Native HTTP now emits fixed response-head facts linked to its existing send
counter before local encoding/size checks. Production and test compilation pass;
all seven projection and seven existing send cases pass. Compilation peaked at
539,000,832 bytes; the test run peaked at 14,729,216 bytes. No swap or limit events
occurred, and all qualification process groups exited. Request construction,
acceptance and cleanup are unchanged. These facts cannot explain R14's completed
403 response, and none of these source updates changes its frozen runtime.
The Node/JSDOM constructor distinction was subsequently qualified by the three
realm cases below; that measurement is complete.
The separate attempt-path preflight now passes all 14 synthetic cases, including
the exact stale R13 filename pattern behind R14's interrupted handoff. It parses
source without executing it and remains a preparation tool. Peak memory was
16,592,896 bytes. Two additional worker replacement/owner-isolation cases also
pass with the revised 19-case inert suite (47,435,776-byte peak). Both suites had
zero swap or limit events and confirmed process-group cleanup; the unchanged
real-thread results are reused. These checks make no new upstream claim.

An explicit window-realm V2 profile now completes a synthetic SDK operation.
Eight synthetic loader cases pass: hash and encoding rejection, static and
dynamic import refusal, genuine window objects, isolated documents and fresh
module state. The suite peaked at 67,801,088 bytes, with no swap or limit events
and confirmed cleanup. Existing profiles retain Node module evaluation. The new
profile changes where unchanged factory bytes execute; it does not synthesize
constructors or modify navigator values. Three further inert realm cases pass,
confirming Node's missing free constructors and JSDOM's genuine window objects.

The subsequent socket-denied operation minted once in 799 ms and completed in
1,074 ms, preserving device state and joining cleanly. It delivered two local
PNGs, where the earlier Node-realm observation attempted none. Its projection
has 90 fields/567 bytes. Exactly two fields were added, 8/13 and 8/14, both type 5
and length 2; all other structural entries match the prior synthetic observation.
The operation peaked at 116,617,216 bytes with zero swap or limit events. One
diagnostic count was recorded, with zero asynchronous errors or late settlements.
The token was discarded. This establishes local behavior, not upstream acceptance.

Resident bearer preparation now retains the single auth response's expiry and
requires it to cover the remaining control deadline after the existing cancellation
and egress checks. Ordinary bearer behavior is unchanged; no refresh or retry was
added. Both API compilations and 28 targeted cases pass, including the new lifetime
cases and existing resident binding/age cases. This source fix is not deployed.

R14 also reached native HTTP with the explicit dedicated-worker V2 profile.
The normal browser request and paired browser baseline returned HTTP 200
with output. The isolated SDK completed and native HTTP returned complete
403 JSON with Framer's Castle security envelope. Cleanup was confirmed;
the isolated owner remains fenced and the protected service is unchanged.
The live binary, command and configuration matched their bindings. Peak
group memory was 1,699,512,320 bytes, with no swap or memory-limit events.
An initial request-file collision stopped before the paired control. The
existing recovery path sent that previously unsent pair once on the same
resident page, without repeating the successful normal request.

The passive observer captured 104 fields occupying 800 fingerprint bytes
from the accepted browser baseline, with a complete V2 selection window.
Comparison with the prior synthetic SDK observation (88 fields, 563 bytes)
finds 17 browser-only fields, one synthetic-only field and 12 shared fields
with differing types or lengths. This is a comparison across those two
observations, not a capture of R14's SDK token or evidence of a refusal cause.
No raw token or decoded field payload was retained. Type tags can themselves
encode Boolean branches; equal lengths do not establish equal payloads.
Next, use these differences to select source-backed discriminators while
preserving the retained owner and rate history. No repeated request is queued.

R13 tested the native-canvas V2 package through the owned resident control.
The normal browser and paired browser baseline returned HTTP 200 with output.
The isolated V2 worker completed, joined, and reached native HTTP, which again
returned complete 403 JSON with Framer's Castle security envelope. Exact
binary, configuration and live command bindings were verified. Cleanup was
confirmed; the resident host remains fenced and the protected service is
unchanged. No retry is queued. Native canvas alone did not establish upstream
acceptance. The full isolated service peaked at 1,801,216,000 bytes with no
swap or memory-limit events, so the whole-service memory target remains open.

V2 window construction now uses the existing bounded inline-PNG loader. All
30 focused cases pass, including a real native canvas-to-PNG image round trip.
The single subsequent socket-denied synthetic V2 observation completed in
1,241 ms (1,003 ms in the token method), with unchanged device state, joined
child and discarded token. Peak cgroup memory was 111,439,872 bytes (106.3 MiB),
with zero swap or memory-limit events. The qualified observer recorded zero
resource attempts, deliveries or refusals: this SDK call never used the new
loader. A source follow-up found that this initial integration also changed
navigator.userAgent through the shared loader default. V2 now retains its
original JSDOM default; the existing V3 user agent remains unchanged. Fourteen
focused checks pass, including both user-agent contracts and real PNG decoding.
The corrected synthetic observation again recorded zero resource attempts. It
completed in 1,229 ms (998 ms in the method), peaked at 110,182,400 bytes
(105.1 MiB), preserved device state and joined cleanly with zero swap or limit
events. Its token was discarded. The first result remains preserved with its
user-agent difference explicitly recorded. Neither result justifies a live
retry by itself. Next, use the existing V2 comparison evidence to select another
concrete runtime difference, preserving R13 and its retained owner.

A separate socket-denied observation of the current V2 runtime recorded its
initial readiness callback with setup count zero and asynchronous count one.
The token method completed once, device state was preserved, and the child
joined. The debugger-instrumented group peaked at 130,375,680 bytes with no
swap or memory-limit events; its timing is not a performance benchmark.
Source tracing explains a possible counter residue: the first dedicated-worker
registration expects two replies, while its handled unavailable-worker path
reports one failure. The counter alone therefore does not prove work is still
running or justify a warm-up delay. An isolated source task now prepares a
dormant real Node worker adapter for bounded qualification. No live retry or
change to ordinary serving is queued, and this does not identify R13's cause.
The operation now supports a worker-owner cleanup hook before its final device
check and window closure. All 25 operation cases pass, including blocked join,
failed join and identity drift during join. The hook remains dormant until a
runtime supplies a worker owner; no new SDK call was used to validate it.
The explicit dedicated-worker profile passes 13 inert cases and two real
Node-thread cases, with a 72,491,008-byte peak and zero swap or limit events.
One subsequent socket-denied synthetic SDK operation also passed: one token
call, unchanged device, one joined worker with two replies, and no worker
errors. The operation completed in 985 ms (729 ms in the token method), with
a 119,996,416-byte group peak under the unchanged 160 MiB cap. No swap or
limit events occurred; the child joined and the token was discarded. PNG
resource attempts remained zero. These single-run timings are observations,
not a latency benchmark. Ordinary V2 construction and R13 remain unchanged;
this result proves local completion, not upstream acceptance.

The current serializer uses wire format 13 and five fingerprint groups.
An isolated diagnostic outer-frame decoder now passes seven synthetic cases,
including a frame matching the public castle-rs golden digest. It returns
private bytes to its caller, has no logging and is not connected to serving.
The report projection now passes eleven focused synthetic cases, including
duplicate and escaped JSON keys, malformed metadata, structural accounting,
fixed-error privacy and input-buffer clearing. The isolated suite peaked at
17,055,744 bytes with zero swap or limit events; its process group is gone.
The prior seven frame-decoder cases are reused unchanged. The five-group parser
is integrated and passes all fifteen synthetic cases. Two further composition
cases pass across frame decoding, group parsing and projection, including the
unparsed event/trailer boundary. That suite peaked at 17,018,880 bytes with
zero swap or limit events and confirmed process-group cleanup.

One subsequent socket-denied synthetic SDK observation passed through the
private diagnostic pipe. It parsed 88 fields across groups 0/4/7/8/9, consuming
563 fingerprint bytes. Only group ID, field ID, type and encoded byte length
were retained. The token was discarded; no field values or event payloads were
saved. The unchanged dedicated profile minted once in 730 ms, completed its
operation in 983 ms, preserved device state and joined its child. Peak cgroup
memory was 122,073,088 bytes under the existing 160 MiB cap, with zero swap or
limit events. These observations validate local structural decoding, not
upstream acceptance. No account, browser, service or Framer request was made.
Next, obtain the same bounded structural evidence from an owned working-browser
mint before drawing conclusions about differences. R13 remains the latest live
result; its retained owner and rate history are unchanged.
The explicit dedicated resident worker entry and its immutable source package
are prepared but have not run through the resident control. Five inert hook
cases pass for observing the original browser mint without replacing its
result, failure or cancellation. The passive browser observer passes all
thirteen synthetic cases, including invalid context, decoder failure, copy
isolation and token privacy. Its isolated suite peaked at 17,244,160 bytes with
no swap or limit events and confirmed process-group cleanup. The diagnostic
host and immutable R14 launch files are prepared with the qualified dedicated
worker profile. R14 subsequently ran under the existing runtime ownership and
rate checks, with the concrete result recorded above.
A subsequent observer correction rejects disguised string-subclass keys.
All fourteen synthetic cases pass, with a 17,362,944-byte peak, zero swap or
limit events and confirmed cleanup. This is qualified source for future
packages; R14's frozen host and recorded result are unchanged.

R12 completed an isolated-V2 request that was refused. The normal browser
and paired browser baseline returned complete HTTP 200 model output. The
isolated V2 component completed under the observed SDK identity, its child
joined, and native HTTP returned complete 403 JSON with Framer's Castle
security envelope. Cleanup was confirmed and the resident host remains fenced.
The explicit V2 configuration and exact binary were verified against the live
launch. No retry was sent; the protected service remains unchanged. This closes
the local setup gap, but browserless upstream acceptance remains unresolved.
The next local experiment used the existing native-canvas DOM and a synthetic
device, with every socket denied and the token discarded. V2 completed with
zero diagnostics, but the first run hit its 160 MiB memory cap. The V2 input
verifier still allocated entire native libraries for hashing. It now streams
through a 64 KiB buffer, preserving the exact hashes and failure behavior.
All 26 focused hashing and operation cases pass. With this sole execution
change, the second canvas run peaked at 108,212,224 bytes (103.2 MiB), with no
memory-limit events or swap. It minted once in 1,033 ms, preserved the device,
and joined the child. The failed first result remains preserved. This is local
component evidence, not upstream acceptance or whole-service memory evidence.
R13 used the explicitly packaged canvas V2 worker after the existing owned
retirement, rate-history and account checks. Its refusal is recorded above.

The R11 normal-browser proof returned local HTTP 500 without model output,
so the paired baseline and isolated-V2/native leg did not run. Its no-status
record cannot distinguish a failed fetch from a failed response-body read.
On `fix/ai-post-failure-phase`, AI POST failures now retain validated phase
and observed header-status diagnostics in the existing event, linked by job
ID and stripped before the reply. Status `-1`, retries, rate accounting and
provenance rules are preserved. Five actual-script JS cases, six host/event
cases and 21 existing provenance cases pass with networking disabled. No live
request was repeated during those tests. The diagnostics do not establish the
cause of R11. Both diagnostics are now integrated on `browserless`.
The RPC observer emits only fixed method/category labels and preserves the
original failure and uncertainty fence. Its five new tests and 31 existing
activity/ownership cases pass. The unchanged Rust executable remains reusable;
R12 used the explicitly packaged sidecar, with no hot reload.

R10 returned complete HTTP 200 from the normal browser and paired baseline,
then stopped locally because the isolated V2 worker observed changed device
storage. Native HTTP was never invoked; cleanup was confirmed. Saved evidence
shows the supplied device is the original hyphenated UUID, which the selected
V2 SDK does not accept. No SDK identity was available in the old host's cache.
The resident source now captures a separate observed V2 warmup identity and
binds it through worker admission and post-join verification. It preserves
the current HTTP cookie/device and refuses missing or replaced metadata.
All 71 focused Python cases, 18 Rust coordinator/cancellation cases, and five
JavaScript fixtures pass, including actual decorated host warm/mint strings.
The first new JS fixture lacked a completion watchdog; its incomplete result
is preserved and superseded by the explicit completed run. The production
daemon also links. Isolated-owner activation is next; a live V2 mint using
this identity binding remains unqualified.

The resident SDK coordinator now explicitly signals worker cancellation even
when cancellation wins before the result future's first poll. Both focused
regressions pass: no child starts for a pending launch, and a started inert
child is reaped promptly. The fixtures also drain the original completion
before asserting their result. Current main test compilation passes, using
the already validated unchanged libraries. This correction is not deployed
in the queued R10 control, whose frozen executable and inputs remain unchanged.
The shared ordinary-serving adapter and V2 selector remain separate candidates.

R9 completed its normal browser request with HTTP 200 and model output, then
refused the crossover because current-document provenance was unavailable.
It sent no baseline, SDK mint or native request. The generic bootstrap error
left Rust cleanup unconfirmed even though the Python constructor had acquired
no lease or pending work. A new exact, identity-bound unavailable receipt
covers only that pre-acquisition provenance refusal. Later failures retain
uncertainty. Twelve focused Python and 36 host tests pass; host/API libraries
and the production daemon compile. R9's original failure remains preserved;
the correction awaits replacement-owner activation and a new control.

The R8 live control established accepted, complete HTTP 200 from the normal
browser and its paired baseline. The isolated worker then rejected input
before running the V2 component, so native HTTP was not invoked. Cleanup was
confirmed and the protected service stayed unchanged. The current page was
Framer's `/projects/` dashboard, which the resident worker's URL predicate
incorrectly excluded. The explicit resident V2 entry now accepts that exact
path, preserving the supplied URL and provenance-owned project authority.
Ordinary worker validation remains unchanged. All 47 focused JavaScript cases
pass, including the dashboard regression and origin/path/device refusals.
The corrected worker bundle still needs owned activation and a new matched
control; R8 itself provides no isolated-SDK or upstream-acceptance evidence.

The retained resident control now has an explicit V2 worker path in source.
It charges the existing rate owner before worker launch, joins the child, and
checks the same document and jar afterward without another browser mint.
It keeps the original bearer, current jar, egress and worker timestamp.
There is no fallback to a browser mint after a V2 failure. The changed host
and API libraries compile. All 68 focused Python cases pass; the three unchanged
embedded-JavaScript fixtures reuse their exact prior passing source/result.
The latest aggregate rerun did not complete one unchanged JavaScript VM fixture,
so it is not reported as a passing aggregate run. Rust fixtures, final linking,
the added V2 login-URL fixtures and live activation remained pending at that point.

The integration subsequently passed all 34 host, 18 API and 60 coordinator,
output, socket and startup cases. The production daemon links. The initial main
compile found an unavailable `tracing` dependency in diagnostic calls; these now
use the existing fixed-field event logger. The passed library phases were reused.
All 21 V2 component/URL, six project-URL and 19 worker-source JavaScript cases
also pass with networking isolated. Current source is ready for owned activation;
no account-bound isolated V2 token has yet been sent to upstream.

The separate V2 worker now passes private token transport and joined-child
qualification. One socket-denied synthetic SDK invocation completed its method
in 604 ms, component in 1,210 ms, and private handoff in 6 ms. Conservative token
age was 1,272 ms; the token was discarded. The owned group peaked at 129,159,168
bytes with no swap or limit events, and the runner confirmed unit removal.
All 57 Rust owner cases and eight JavaScript handoff cases pass. The V3 worker
keeps its existing acceptance rules; V2 has a separate exact decoder and fixed
failure categories. This does not establish account-bound upstream acceptance.
The adapter's new diagnostic match arm awaits its next compilation.
Next, bind V2 to the retained resident lease with pre-worker rate admission and
post-join context verification, preserving the current jar, bearer and original
worker timestamp. No running browser has been changed.

A separate direct V2 component now completes one actual unchanged SDK call
with a synthetic UUID-shaped saved device. The method took 597 ms and the
component took 1,142 ms. Storage and cookie identity remained unchanged;
the window closed and the child exited and was reaped without forced cleanup.
All sockets were denied, no account was loaded, and the token was discarded.
The owned group peaked at 88,027,136 bytes with no swap or limit events.
Nineteen nonfatal DOM diagnostic events were recorded; asynchronous faults
and late settlements were zero. This proves component completion, not native
HTTP acceptance or a production memory reduction.

The earlier all-`a` synthetic device failed the storage check. That value
does not satisfy the SDK's UUID-shaped validation, but the earlier result
does not identify the exact failing read. Fixed device-failure categories now
distinguish local storage reads/values and cookie reads/counts/values without
exporting their contents. All 19 focused component fixtures pass. The SDK,
identity guards, method count and deadlines are unchanged. The next step is
private, age-preserving transport into an account-owned native preparation.
The component is not connected to serving, and V3 acceptance is unchanged.

Failed V3 worker exits now retain an allowlisted stage only after the child
joins. Cancellation, deadline precedence and success validation are unchanged.
That Rust integration is source-only and awaits its focused compilation/tests.

The offline worker now accepts the same bound project access-link URLs as
the native credential target. Six new URL cases and 19 existing worker-source
cases pass with networking disabled. The original URL and all origin, project,
route and device checks remain intact. This corrects a local pre-SDK refusal;
it does not explain the earlier completed HTTP 403 responses. No live worker
bundle has been updated; a future bundle must bind the changed source hashes.

The next owned control, R7, also succeeded on October 24. Its browser baseline
returned accepted HTTP 200, and native HTTP returned 200 with completed,
nonempty model text. Cleanup was confirmed. Both mint logger windows had
complete coverage and recorded one V2 selection, zero V3 selections and zero
fixed failure counters. This proves the observed successful V2 crossover;
it does not explain prior offline V3 refusals or attest browserless minting.
The isolated owner remains resident and fenced; the protected service stayed
unchanged. A local request-filename collision occurred after the normal request
and before any control send. The unsent control then ran through the same
owner on its successful resident page, without repeating normal inference.
Streaming, tools, browserless token generation and memory targets remain open.

The native report now includes `native_output_complete`, requiring bounded
response EOF, HTTP 200, JSON and the same route-specific completion/nonempty
text fields as the browser baseline. Seven output cases, the socket schema
case and all 13 private-client cases pass; the updated daemon links. No response
content is exported. This fact is independent of cleanup, and the matching
14-field client must be used. The prior R6 result remains JSON-object evidence
only. The new executable reuses the validated unchanged Rust libraries;
activation must select the explicit current sidecar because the host library's
embedded Python predates these updates. R7 used this update only in the
isolated owner described above.

Resident mint-branch diagnostics now pass 59 focused Python cases and three
JavaScript fixture scripts. They read the existing wrapper's matching cached
logger and emit only fixed counters, window coverage, phase and lease ID.
Observer failures do not change token validation or mint fallback behavior.
R7 used these diagnostics and the native-output report in its supported
isolated owner. The earlier R6 control below cannot establish V2 or V3;
its report had no branch observation. The R7 owner remains resident and
fenced, with no hot reload or duplicate request.

The matched resident browser/native control succeeded at 15:55 UTC on October
24. Normal browser inference produced model output; the paired browser baseline
was accepted with HTTP 200, and a separately minted fresh token from that same
context was accepted by native HTTP with status 200. Both bodies completed and
cleanup was confirmed. The native report certifies a JSON object, not parsed
model output or streaming. The isolated owner remains resident and fenced;
the protected service stayed unchanged. Browserless token minting and the
full streaming, tool, latency and memory objectives remain unproven.

An isolated resident-enabled owner now runs the validated binary and explicit
sidecar bundle. One normal browser request returned HTTP 200 with model output.
The subsequent crossover stopped before its baseline or native request. A
read-only observation found no retained provenance, active Python commands,
pending RPCs or resident lease. The installed Playwright callback mapper
reproduces the cause: it writes callback metadata onto the slotted provenance
object, raising AttributeError during listener registration. Plain-function
listeners correct this without changing identity or navigation guards.
All 21 existing provenance cases and three real-Playwright callback cases pass.
The supplied cookie-order fix is integrated as well. It compares validated
snapshots in stable name order while retaining the original wire header and
duplicate-name order. Its seven cases, 29 crossover cases and 11 bootstrap
cases pass together. Changed values, membership and duplicate precedence
still refuse. This closes a separate possible setup refusal, not the cause
of the observed callback failure.
One initial new fixture lacked a cooldown constant; its response was changed
to HTTP 500 to test listener cleanup without exercising unrelated cooldowns.
Fixed bootstrap reason diagnostics also pass 11 focused inert cases. These
changes are deployed only in the isolated owner used for the successful
control above. The old owner was retired after verified idle capture, with
its current rate history restored to the new owner. The protected
browser service remains unchanged; no SDK cause has been established.

The resident admission fix now rejects competing ordinary requests before
their browser-status probe. It retains the existing account-management guard
through cleanup; genuine status failures still retain their uncertainty fence.
The current core and focused server fixture compile, and all eight selected
admission tests pass, including cancellation, cleanup and negative-status cases.
The unchanged Python r3 bundle also passes all 130 cases and three JavaScript
scripts with zero swap or limit events and confirmed runner cleanup. The current
host and API libraries, 17 coordinator/adapter cases, 33 host cases, 17 API cases,
34 socket/startup cases, and the linked production daemon now pass validation.
The ten private-client cases also pass. The first API archive build hit the
32 MiB file limit; omitting unused LLVM bitcode produced passing API and server
libraries without raising limits. Earlier failed evidence remains preserved.
The successful isolated control is described above. The protected running host
has no mechanism to activate this protocol in its existing process.

Fresh r3 validation bundles now bind the current production source: 15
coordinator plus two adapter cases, 31 socket plus three CLI cases, and 130
Python cases plus three JavaScript fixture scripts. Preparation, exact source
binding and inventory checks pass. The core compiler phase is prepared but
unlaunched. At 15:07 UTC on October 24, available memory was 335,552,512 bytes
with no free swap; its unchanged requirement is 2,741,140,684 bytes. A separate
library snapshot now prepares all 33 host and 17 resident API unit cases,
preserving the selected test files and the r3 bundles. It requires successful
current dependency builds before preparing each compile/run phase. No test,
build, browser call or deployment ran for these milestones.

Resident fast-403 cooling now survives document invalidation after the fetch.
The update requires the original lease, account, lifecycle, browser/device and
history/token containers; replacement state receives no late mutation. The
existing timing predicate and refusal classification are unchanged. Four
supplied inert cases are integrated, with a fifth covering token-pool
replacement and the navigation case strengthened to use the real listener.
All five remain unrun; source syntax and whitespace checks pass.

Resident request numbers now stay within JavaScript's safe integer magnitude,
including decimal and exponent forms. One Rust body validator is reused by
socket parsing, launch admission and final-body preflight; the Python baseline
boundary applies the same limit. Values are refused rather than rounded.
The two supplied parser regressions are integrated, bringing socket coverage
to 31 source cases. Existing host/Python cases cover nested limits and ordinary
fractions. These changes are uncompiled and unrun and do not explain any saved
HTTP refusal.

Resident baseline bodies are now checked after turn-ID substitution and
before lease acquisition. An explicit local `InvalidBaseline` preserves
previously confirmed preparation cleanup; other RPC errors remain unconfirmed.
The size-boundary fixture now covers turn-ID expansion, and a new native-owner
fixture checks the resulting account closure behavior. These source changes
are uncompiled and unrun. The coordinator inventory is now 15 plus two adapter
cases; new validation snapshots must use that inventory.

Resident baseline replies now retain the existing refusal classification as
fixed policy/basis labels. The strict Rust decoder validates those labels and
records them before rejecting the baseline, allowing the existing risk-history
publisher to apply its normal policy. This does not attribute the original
HTTP 403 to an SDK defect. Both supplied five-case navigation/refusal fixtures
are integrated; existing Rust and Python cases cover the new field and reject
malformed replies. Syntax and whitespace checks pass; execution is pending.
Earlier r2 validation snapshots predate this change and must not be run as
current evidence. Local device cooling after document invalidation is now
corrected in source as described above, with execution still pending.

Current-main validation is now prepared in a separate immutable local snapshot.
It covers all 29 socket and three CLI cases plus a linked daemon build, reusing
only verified builds from the current dependency graph. Preparation syntax,
exact fixture inventories and preservation of the earlier frozen inputs pass.
No compiler or test ran. At 14:55 UTC on October 24, available host memory was
260,587,520 bytes; the first compiler phase requires 2,741,140,684 bytes under
the existing reserves. Its launcher was not retried. The next executable step
remains the prepared core build once sufficient memory is available.

A subsequent socket review found that the short internal bind alias could
hide an unusably long client path. Startup now rejects original paths beyond
Linux's 107-byte limit. The new case covers ASCII and multibyte paths; the
positive case connects through the actual public path. Source review passes,
but execution is pending. Refreshed r2 snapshots preserve the earlier inputs
and give the main fixture a private, short temporary path inside its sandbox.

The private resident socket and optional daemon startup wiring are now
integrated in source. The native-dispatch build accepts an explicit local
socket with the Camoufox engine and refuses simultaneous offline ownership.
The socket has 29 inert cases and the CLI binding has three, all uncompiled
and unrun. Client/server field sets and the frame cap match in a static check;
this is not executable protocol validation or a live crossover.

A source review found that navigation after a settled baseline fetch could
erase its observed HTTP status when the context check refused. The resident
operation now preserves validated fetch facts before propagating that refusal.
Acceptance stays false and native mint remains fenced. This correction passes
syntax and whitespace checks but is not executed; the independent navigation
regression fixture is pending integration.

A private one-request client is now implemented for the resident socket. It
validates the local socket owner and the fixed report schema, sends no second
request, and reports lost or partial exchanges as unknown. Its ten inert
fixtures are frozen but unrun: the 128 MiB runner was refused before execution
by the existing memory reserve. Python syntax checks pass. This does not
activate the socket or run a token crossover. Usage and result semantics are
in [resident-control.md](resident-control.md).

The Python version 2 resident protocol is now implemented in source. Acquire
captures context without minting; the baseline uses one cached-wrapper token;
only an accepted baseline enables a separate native-token mint. One retained
lease, exact context checks and the original deadline cover the sequence.
The current frozen bundle contains 115 Python cases and three exact-JavaScript
fixture scripts, including the 29 new crossover cases. Its October 24 launch
was refused before execution by the existing protected-memory reserve. No
test process started, and no pass is claimed for the new source.
Version 1 remains available with its original acquire behavior. Chat has no
accepted completion rule in this protocol and refuses continuation. Rust
coordination is now implemented with an asynchronous bootstrap adapter and
explicit future-startup activation. The adapter installs one
shared session boundary and opens an exclusive native generation only after
verified browser metadata. The Rust worker reserves application ownership
before capture and joins capture, browser and native work through cancellation.
Its 14 coordinator cases and two adapter cases are uncompiled and unrun.

Resident bootstrap is implemented in source. It reads the retained successful
browser's metadata through the existing owner, checks the exact sidecar and
proxy-registration generation, and returns no token or cookie header. The
Python operation owns its read-only RPCs through caller cancellation. Nine
new Python cases are frozen but unrun: the 128 MiB test runner was refused
before execution by the unchanged protected-memory reserve. Five new Rust
cases are also unrun. Python syntax and whitespace checks pass; rustfmt is
not installed. This does not activate the running host or prove a crossover.
The asynchronous startup-owned capture adapter and version 2
baseline/native-mint sequence are now implemented in source, as described
above. Document-continuity hardening is now
integrated from the supplied patch: listeners start before the send, and
main-frame navigation, including same-URL reload, invalidates provenance.
Refusal, failure and cancellation detach unfinished observations. The updated
21-case provenance fixture and its shared fake-page adaptation are unrun.

The private resident Python operation and its host activity integration are
implemented. Saved isolated checks pass for 26 host activity cases, 25 private
operation cases and two exact-JavaScript fixture scripts, 141 existing host
regressions, and 10 runtime packaging cases. These are inert checks, not a live
token crossover. A subsequent review found that a cancelled browser launch
could complete without a cleanup receipt. The receipt now belongs to the
retained launch task. Five added cases bring that module to 31 cases; the new
run was refused before execution by memory admission, so the correction is
unvalidated. The original 26-case result remains evidence for the earlier source.

The Rust startup binding has 19 focused cases, currently unrun. The executable
build and a smaller metadata-only check were refused before execution by the
unchanged memory reserve. No compiler or test pass is claimed for this binding.
The retained daemon worker remains source-only. It now uses verified bootstrap,
snapshot-only acquire, one bearer, the browser baseline and a separate native
mint under one retained lease. Startup/socket integration is source-complete;
executable validation remains pending. Existing serving processes remain untouched.

An additive resident mint ticket is now implemented in source with three new
inert cases, all unrun. It rechecks the retained bearer and current jar before
the native mint, timestamps that operation before IPC, and consumes the ticket
after post-mint checks. Earlier API results below cover the published 14-case
version; they do not validate this addition or its new coordinator wiring.

The resident API binding compiles and passes all 14 focused inert cases.
It checks the captured page, lease identity, current jar and launch route,
retains one bearer for both arms, and preserves token age from before acquire.
Test peak was 13,635,584 bytes with zero swap or limit events and confirmed
runner removal. Earlier build admission/cap failures remain recorded. The
successful build required a narrow expected-source correction for an excluded
old loopback fixture. These checks do not establish a live owner handoff or
upstream acceptance. Host and daemon integration remain separate work.

The generation-bound resident RPC client compiles and passes all 10 focused
inert tests. These cover exact frames, missing/replaced/dead owners, ordinary
RPC compatibility and cancellation ownership. Test peak was 12,668,928 bytes,
with zero swap or limit events and confirmed runner removal. The original
full host build was refused before execution by memory admission; a focused
build passed under a 768 MiB cap. No resident command is activated by this
transport helper, and its replies still require owner-side validation.

The browser HTTP-200 provenance helper passes all 16 inert cases in a
network-isolated runner. It binds the actual pre-send objects and invalidates
closed or replaced objects without another mint, browser call or request.
Peak was 31,678,464 bytes, with zero swap or limit events and confirmed runner
removal. No serving process was changed. Resident acquisition still needs the
activity integration and retained daemon-owned handoff.

The working browser service was checked through bounded, sanitized event
samples on October 24: it was minting and completing HTTP 200 inferences.
The running generation remained unchanged. Existing isolated comparisons do
not reproduce its warm device context, wrapper or bearer acquisition, and
both comparison arms omit cookies that the working service includes.
An explicit native resident-browser header option is implemented. It sends
the supplied current jar and the serving host's fixed inference headers;
the default behavior is unchanged. All 26 focused Rust cases pass, including
exact loopback headers and cancellation. Test peak was 15,253,504 bytes with
zero swap or limit events. Three earlier harness failures are preserved;
the successful run reused the exact verified binary and build provenance.
The active daemon has no safe existing resident-token handoff endpoint.
Resident ownership and activation remain separate work; no live crossover
result or SDK attribution is claimed.

Eight inert async cases now pass for the standalone resident activity ledger:
mutual exclusion, retained RPCs after caller cancellation, retained completion
after waiter loss, account-local fencing, replay/capacity refusal and invalid
admission. Peak was 16,465,920 bytes with zero swap or limit events and confirmed
runner removal. Host dispatch/background integration is now implemented as
described above. These tests do not establish a working resident-token handoff.

The comparison now records bounded SDK branch/failure counters without token
or error contents. Its inference headers use the public caller's default
`application/json` and omit `X-Requested-By`. All 106 focused Python cases and
30 exact-JavaScript cases pass in a network-isolated, memory-capped runner.
Two initial fixture failures (data-URL quoting and module identity) remain
recorded; production code did not change to satisfy them. Logger eviction and
concurrent page activity limit what the counters prove. R16 remains a refused
browser baseline; these changes have not had another live test.
Native inference now uses the same JSON header and omits `X-Requested-By`.
Both Responses routes also omit the redundant outer `model`, keeping the
required model inside `request`. Messages retain their existing envelope.
The API and comparison executable compile; all 24 focused Rust cases pass,
including actual loopback headers, authentication, cancellation, current-jar
ownership and the new envelope cases. The first focused build failed because
the test-selection script omitted complementary cfg branches; its corrected
snapshot passed without a production change. The passive app AI-policy
preflight is now integrated. All 136 focused Python and 58 JavaScript cases
pass, including personal spaces without organization configuration. One stale
realm-test expectation was corrected to include the new policy evaluation;
the original failed result is retained.
R17 then reached editor readiness but failed at `policy_rpc_timeout`, before
bearer acquisition, mint or either inference request. Account and browser
cleanup both completed; the child exited naturally and its group was absent.
Peak group memory was 1,280,491,520 bytes with zero swap or limit events.
The protected browser was unchanged and the isolated owner remains parked
with its full cooldown history. The policy result is unknown.
Its finalizer now parks both comparison outcomes instead of
starting the older staged daemon on success. All 28 inert finalization cases
pass, including refusal of incomplete cleanup and mismatched history. The first
test launch used a relative path and did not reach the fixture; the corrected
absolute-path run passed. Standalone and offline-worker validation remain open.
Seven additional controller cases verify that the outer result reports success
only after the paired result and preserved history succeed. Missing restoration,
publication failure or a changed protected browser still reports failure.
The corrected standalone daemon now compiles against the validated API and
current observer. Its retained core, server, store and worker library sources
match the current source. Build peak was 480,161,792 bytes with zero swap or
limit events. Two isolated CLI startup checks pass. Candidate SHA-256 starts
`688cec05`; it is not installed or running. Authenticated offline delivery and
streaming validation remain open.

A network-disabled browser probe ran the exact readiness and policy code
against synthetic modules and page state. The cached-module case completed in
8.2 ms; first-time imports completed in 62.8 ms. Both returned an allowed policy,
retained no pending RPC and closed normally. Peak group memory was 553,127,936
bytes with zero swap or limit events. This qualifies the basic evaluation path
in that fixture, not the real page's workload or the cause of R17's timeout.
The policy now emits up to seven fixed progress labels without changing its
deadline or decision. All 140 focused Python and 61 JavaScript cases pass.
An isolated browser also delivered the full sequence on success and stopped
at the correct import label in two deliberately stalled cases. Both timeouts
kept the RPC owned until context closure; all three contexts closed cleanly.
Peak group memory was 593,379,328 bytes with no swap or limit events.
R18 then reached evaluation entry, loaded the engine module and timed out while
importing the shared policy module. It never reached the policy getter or either
inference leg. Both cleanup receipts completed; peak group memory was
1,098,117,120 bytes, with zero swap or limit events. The protected browser was
unchanged and the isolated owner is parked with its full history.
The fresh public entry now selects `web.MNQWRMGH.mjs`, policy
`chunk-3W5IVLGF.mjs` and wrapper `chunk-WA4HVE2H.mjs`. Our old module URLs still
served valid files, but belonged to the earlier graph. The policy getter and
wrapper body are unchanged; their asset dependencies changed. Comparison pins
now follow the new selection. The host, API and comparison executable build
successfully, and all 140 Python plus 61 JavaScript checks pass. Build peaks
were 403,652,608, 663,179,264 and 450,957,312 bytes respectively, with zero swap
or limit events and absent groups.
R19 then failed at `editor_wait_timeout`, before policy evaluation or SDK use.
The diagnostic read matched the project URL but found no history object or
device storage. Neither inference leg ran. Both cleanup receipts completed;
the child exited naturally, its group was absent, and peak usage was
1,001,054,208 bytes with zero swap or limit events. The protected browser was
unchanged and the isolated owner remains parked with its full history.
The new saved router and web entry retain the old local readiness logic;
their dependency URLs changed. This does not establish the actual page's
loaded graph or explain the earlier HTTP 403s.

Passive startup diagnostics now count the owned page's document events,
public module requests, failures and pending loads, plus JavaScript errors.
Only restricted public asset basenames and fixed fields leave the observer.
All 149 focused Python cases pass. The isolated browser fixture also passes,
distinguishing pending, HTTP-failed and network-failed modules, document
events and a JavaScript error. It closed cleanly at a 513,777,664-byte peak,
with zero swap or limit events. Two earlier probe failures exposed a callback
signature error in these new diagnostics; the corrected handlers accept the
page argument and the fixture now reproduces it. No acceptance, deadline or
retry behavior changed. Next, build the observer into the bounded comparison.

The observer host, API and comparison executable then built successfully.
R20 loaded the current web entry and 456 observed public modules without
asset failures or JavaScript errors. The policy check returned known and
enabled. The baseline then refused at `baseline_header_before_mint_values`:
its current cookie snapshot differed from the snapshot used for its bearer.
It made zero inference sends and never reached comparison minting. Both
cleanup receipts completed; the child exited naturally with its group absent.
Peak usage was 1,239,252,992 bytes with zero swap or limit events. The protected
browser stayed unchanged and the isolated owner is parked with full history.
Source provenance shows that the supplied header was the original context
reply. The fresh snapshot passed exact session and device checks, so this
mismatch involves another cookie name. Its identity and writer are unknown;
no cookie contents were exported. Neither Castle nor native HTTP is implicated
by this local refusal.
An initial full-cookie settling step now passes all 154 focused Python cases.
It requires one second of equal snapshots within five seconds, with stable
page, device and login. Later full-header comparisons remain exact. This is
a bounded startup measure, not proof that cookies will remain unchanged.
The first run had one fixture expectation failure: an existing snapshot
guard caught injected drift before the new sampler did. The corrected case
retains refusal and cleanup assertions. Browser and JavaScript evidence is
reused for unchanged operations; no later authenticated result is claimed.

A traced offline SDK probe identified the browser crash: PulseAudio requests a
64 MiB shared-memory file, but the comparison runner allowed only 32 MiB.
`ftruncate` returned `EFBIG`, followed by SIGXFSZ. The corrected tracing probe
uses the pinned Linux `camoufox-bin` executable. Its predecessor's wrong-path
assertion and the original failures remain preserved.

At exactly 64 MiB the browser closed normally, with no file-size failure,
zero memory/task-limit events and no swap. Peak group usage was 841,531,392
bytes and 175 tasks. The mint result still failed its later `post_context`
binding check; accepted minting and authenticated delivery are not established.
All probes use saved public modules, an empty context and disabled networking.
Buffered wrapper logs now explain the context mismatch in this offline case:
V3 token generation errors, then the wrapper falls back to V2. V2 rejects
the initial hyphenated UUID and generates a different 32-character identity.
A second local token was also V2, so it is not V3 acceptance. Removing syscall
tracing did not prevent the failure. Page-console silence was not useful;
the existing logger buffer retains branch records even at its default level.
The original V3 error is now localized: Firefox rejected TypedArray access
across the automation realm's Xray wrapper. In offline probe R22, Camoufox's
documented main-world evaluation completed one V3 mint with no V2 fallback,
SDK errors, or identity change. The public fixture configuration was placed
in that same realm; no SDK bytes were changed. The browser exited normally,
its group was absent, peak group usage was 833,970,176 bytes with 176 tasks,
and recorded swap and limit events were zero. These findings qualify the
local caller correction, not the cause of earlier HTTP 403s. The correction
is now integrated only for the reserved comparison browser. All 148 focused
Python cases pass, including ordinary authorization, cleanup, cookie checks
and six new realm-selection cases.

Live comparison R12 was killed at its 1.5 GiB child memory limit before it
saved a result. Request count and remote outcome remain unknown. Same-boot
system records and process-group absence establish physical closure; they do
not establish application cleanup or delivery. The protected browser's
generation was unchanged. The isolated service stays stopped, and the full
reserved history was preserved and extended through physical closure.
The kernel selected the browser's WebExtensions process, with about 612 MiB
anonymous resident memory. Offline R23 excluded default uBlock using the
supported launch option. Its unchanged SDK completed one V3 mint with stable
identity, no fallback or error, and clean teardown. Group peak was 614,293,504
bytes, compared with R22's 833,970,176 bytes, about 210 MiB lower in these two
empty-fixture samples. This does not establish the real page's peak.

The comparison-only exclusion is integrated and all 149 focused inert cases
pass. Ordinary browsers and explicit prepared addons retain their settings. A fresh
signed-out public login page also loaded, remained open for 20 seconds and
closed cleanly, peaking at 683,372,544 bytes with zero swap or limit events.
That test supplied no account and blocked AI endpoints. The authenticated
project page's memory remains unverified. R12's surviving command log records
a post-mint context-check failure before the later memory kill, so the two
failures remain separate. The post-mint diagnostic now distinguishes page
drift from device drift without weakening either check. All 61 focused Python
cases and six exact-JavaScript inert cases pass. Check exceptions still retain
the generic post-context stage, and no identity or token data is exposed. The
next bounded comparison uses these diagnostics and the qualified addon exclusion
after the full uncertainty cooldown; authenticated delivery is still unverified.

The next isolated trial, R13, completed without a memory-limit event. It
refused locally at `mint_post_page`: the page URL changed during the mint.
The report records zero baseline inference starts and no native attempt.
Account and browser cleanup completed, the child exited naturally, and its
group was removed. Peak group usage was 1,036,296,192 bytes with 195 tasks;
swap and limit events were zero. The protected browser was unchanged. The
outside finisher kept the isolated service stopped and preserved the full
cooldown history. This trial does not establish an SDK token or HTTP defect.

Preparation now waits for the fresh editor page to publish its initial
canvas history state before binding the exact URL. Saved current application
source places this publication after the initial URL and query rewrite.
The check also requires the expected project and existing device storage;
all later equality guards stay strict. It applies only to the fresh page,
not restored history, and does not guarantee permanent URL stability.
All 84 focused Python cases and eight exact-JavaScript cases pass with
networking disabled. The tests cover delayed binding, timeout/cancellation,
wrong projects, incomplete/transient history and existing mint/cleanup paths.
The first live use, R14, stopped at editor readiness before returning its
initial context. No baseline or native result was produced. Browser cleanup
was confirmed; account cleanup was not, so the original `Cleanup` failure
and isolation fence remain. The child exited naturally, its group was removed,
and peak usage was 1,062,060,032 bytes with 194 tasks and no swap or limit
events. Exact system records establish physical closure and the full reserved
cooldown was extended through that closure. The protected browser is unchanged.
The exact predicate then passed in a network-disabled real browser with a
synthetic page: both JavaScript realms saw the page-authored history and the
normal readiness wait/disposal completed. This excludes basic field-access
incompatibility in that fixture. It does not identify R14's failure: its single
stage still covers predicate waiting, browser RPC, handle disposal and scope
checks. The integrated diagnostics distinguish wait, handle-disposal, scope
and outer-RPC failures. Only a settled wait timeout permits one additional
read-only probe, bounded to one second and eight boolean facts. A successful
probe cannot turn the original timeout into acceptance. All 97 focused Python
cases and 11 exact-JavaScript cases pass with networking disabled, including
cancellation, late completion, malformed replies and diagnostic privacy.
R15 then confirmed a wait timeout: the expected-project URL check was false
while all seven editor-history and device checks were true. Both comparison
legs remained absent. The child exited naturally, browser closure was confirmed,
and the same native preparation fence retained the logical `Cleanup` result.
Peak group usage was 1,043,345,408 bytes with 193 tasks, zero swap and no limit
events. Physical closure and the full cooldown history were preserved; the
protected browser was unchanged.

Public routing source exposed a separate concrete compatibility bug: Framer
can append a project access token after its ID, but all three local URL
checks required the ID at the end. The correction parses that additional
form and still compares the exact ID. All 126 focused Python and 13 exact-JS
cases pass. All 16 Rust credential-target cases pass as well. The focused
build peaked at 427,413,504 bytes; earlier full test builds stopped at their
1 GiB and 1.5 GiB caps and remain recorded as failures. R15 did not retain the actual URL,
so this source defect is not yet established as that run's cause. The next
step is to finish cleanup validation before another bounded comparison.
The cleanup source is integrated:
failed preparation now waits on one shared, strictly matched browser closure
before reporting settled rejection. Unconfirmed closure still retains the
native fence. The API and updated executable compile successfully. The first
13-case run passed eight cases, including matching closure and a dropped
caller. Three cases lacked a required inert fixture file. Two negative cases
exposed a separate issue: the deliberately retained process owner never
reaped its sidecar after natural exit. The corrected setup and production
reaper now pass all 13 cases. The negative cases verify both actual child
reaping and continued refusal of that account afterward. The matched-close
and dropped-caller cases verify retained work finishes before publication.
The next isolated comparison will use the updated child and URL validation;
authenticated delivery remains unverified.
The matching missing-history control also behaved as expected in a real
browser: both realms returned false, the wait raised `TimeoutError`, and
the browser exited cleanly with no swap or limit events. These two controls
used synthetic pages without networking, SDK code or account data.

Live comparison R16 passed editor readiness and the page, device and cookie
checks around minting. Its browser baseline sent one request and received a
complete 231-byte JSON security refusal, HTTP 403. The native leg was therefore
not sent. Account and browser cleanup both succeeded; the child exited
naturally and its group was removed. Peak usage was 1,221,050,368 bytes with
188 tasks, zero swap and no limit events. The protected browser was unchanged,
and the isolated service remains stopped with its full cooldown history.
This establishes that the isolated browser baseline also fails. It does not
identify an offline SDK or native inference-transport defect. Remaining work
is to compare this baseline's setup and request contract with the normal
browser flow and identify its live SDK branch without exposing token data.

Preparation now records fixed phases immediately through the existing sidecar
log, instead of retaining them only in memory until command completion. All
76 distinct focused Python cases pass, including bounded output, privacy,
pending-mint visibility, cancellation ownership and reading the actual file
sink before a command completes. These records do not certify a send or
cleanup. The change is published and its phase records were retained in R14.
The saved application source rules out both its generic loading latch and a
slug-only URL check as complete readiness signals: the latch has other false
states, and the router also updates selected-scope query state.

The earlier isolated comparison (R10) stopped before minting because its cookie
header differed from the snapshot used to acquire the bearer. No inference
transport started, and native inference did not run. Account and browser
cleanup were confirmed; the child exited naturally and its group was removed.
Peak usage was 1,227,051,008 bytes and 182 tasks, with zero swap or limit events.
The controller preserved the stopped isolated owner and full cooldown history.
The protected browser was unchanged. The result does not distinguish cookie
ordering from changed cookie content and does not explain R9's mint failure.

Comparison snapshots now serialize cookies in stable name order after the
existing validation, preserving duplicate-name precedence and values. Exact
header equality still gates both baseline checks. Fixed diagnostics distinguish
ordering, membership and value changes without exposing cookie data. Ordinary
authorization is unchanged. All 73 focused Python cases pass, including twelve
new ordering cases; the unchanged mint JavaScript reuses its prior browser
validation. Fourteen synthetic tests of R11's stopped-state adoption and
outside finish path also pass. R11 is prepared with the tested helper; its next
live attempt requires local file-limit qualification, the full R10 cooldown,
and fresh account/resource checks.
Authenticated delivery and streaming acceptance remain incomplete.

The earlier isolated comparison (R9) reached the browser mint operation and
failed there before any inference transport started. Its saved stage is
`baseline_mint`; native inference did not run. Account closure was confirmed,
but browser cleanup was not. The child exited naturally and its group was
removed, peaking at 1,366,941,696 bytes and 186 tasks with zero swap or limit
events. A matched same-boot journal resolved physical ownership while keeping
the logical `Cleanup` failure and full cooldown. The protected browser is
unchanged, and the isolated service remains stopped. Inner mint and outer
browser-RPC diagnostics now pass 61 focused Python cases. Eight real-browser
cases also pass with networking disabled and synthetic modules, separating
six JavaScript failure stages from a closed-page RPC failure. That probe
peaked at 681,689,088 bytes and 172 tasks, with zero swap or limit events and
confirmed process-group removal. These results validate diagnostics, not SDK
or authenticated inference. Another live comparison remains pending.

The earlier isolated comparison (R8) passed device readiness and all seven
cookie, login and page checks. It returned a settled baseline preparation
failure with zero inference transport starts; native inference was not
attempted. This does not identify a Castle or HTTP failure. Fixed diagnostic
stages now distinguish the two jar checks, minting, rebinding, fetch setup and
result handling. All 48 targeted comparison and diagnostic cases pass.

R8 confirmed account and browser cleanup. Its child exited naturally and the
group was removed, with a peak of 1,221,373,952 bytes and 179 tasks and zero
recorded swap or limit events. Automatic service restoration then failed at
process inspection inside the test sandbox. An outside recovery armed the
idle instance, and a separate verified shutdown preserved its unused handoff
and all cooldown history. The isolated service is stopped; the protected
browser service is unchanged. The next controller keeps clean refusals
stopped and performs any successful-comparison restoration outside the sandbox.
R9 used that controller and retained its fence after unconfirmed browser
cleanup. Fourteen synthetic adoption and restoration cases now pass. No live
restoration acceptance is claimed from those cases.

A network-isolated browser fixture reproduced a local cookie-selection defect:
the SDK's host-only and domain cookie pair becomes two entries in Playwright's
API-URL selection, although the browser sends only the domain cookie there.
The adapter now applies exact host/domain, path and secure-cookie matching.
The in-page guard accepts identical visible device cookies and still rejects
conflicting values. All 82 affected Python cases and 16 exact-fetch JavaScript
cases pass. The corrected selection and fetch also pass in the real browser
using synthetic cookies and intercepted responses, with no SDK or account.
That run peaked at 528,543,744 bytes, with zero swap/events and group removal.
R8 subsequently passed the live context and cookie checks; inference remains unverified.

The earlier R6 confirmed browser cleanup and natural child exit, with the group removed.
The native account still retained `Cleanup`. Peak usage was 179 tasks and
1,210,380,288 bytes, with zero recorded swap or memory/task-limit events.
The protected browser is unchanged and the isolated account remains fenced.

The earlier isolated comparison (R5) identified a preparation failure: the
project page and login cookie matched, but device storage was not ready at
the immediate post-navigation snapshot. This occurred before controlled
bearer acquisition, token minting or inference. It does not explain earlier
upstream security refusals. The bounded diagnostics retain stages and boolean
checks only; their ten regression and ten privacy cases passed before use.

R5 confirmed browser cleanup, while the native account retained its
`Cleanup` failure. Peak usage was 186 tasks and 1,101,819,904 bytes, with zero
recorded swap or memory/task-limit events. The child exited naturally and its
group was removed. A separate exact-invocation journal check resolved physical
ownership while retaining the logical failure, unknown remote outcome and
full cooldown. The protected browser service remains unchanged; the isolated
service stays stopped and fenced.

The readiness correction waits up to ten seconds for an actual stored device
ID, then disposes the JavaScript handle before validating the original
snapshot. It does not mint, write device state or repeat navigation. All 46
focused Python cases pass, including late readiness, timeout, cancellation,
disposal failure and final context mismatch. The exact helper also passes
two network-isolated tests in a real browser: delayed synthetic storage and
the actual ten-second timeout. That run peaked at 167 tasks and 863,322,112
bytes, with zero swap/events and confirmed group removal. Earlier fixture
failures are preserved. The subsequent R6 result is recorded above; authenticated delivery remains unverified.

The earlier R3 comparison reached browser startup but exhausted its
96-task child limit. It returned no baseline or native result. Both logical
cleanup flags were false; the child exited naturally and its process group
was removed. The isolated account remains stopped and fenced, with the
reservation retained. The protected browser service remained unchanged.

A subsequent blank-browser test passed with a 192-task child inside a
256-task controller. It opened a fresh context and blank page, evaluated a
constant and closed normally. Peak usage was 151 tasks and 524,369,920 bytes,
with no recorded swap or memory/task-limit events. This test had no account,
SDK invocation or external network. The 1.5 GiB child and 2 GiB controller
memory limits are unchanged. Signed-in delivery remains unverified.

The signed-out public login startup now passes with a 256-task child and
320-task controller. It reached the login form, evaluated a constant and
closed the context and browser normally. Peak usage was 178 tasks and
651,210,752 bytes, with no swap or memory/task-limit events. Earlier probe
failures remain recorded. This supplied no account and made no inference
request; it does not establish authenticated delivery.

The isolated comparison child compiles and its boundary tests pass. The 21
coordinator/child cases passed first; two additional browser-refusal cases now
pass, including a dropped caller. The refusal prevents native authorization,
and final publication waits for account and browser cleanup. Four tests of
the real executable also pass: oversized/truncated input, unknown plan fields
and a wrong start nonce. Those tests never start a browser or load an account.

The standalone child build peaked at 253,706,240 bytes. The two additional
Rust tests peaked at 79,204,352 bytes; real-process tests at 21,770,240 bytes.
The saved checks confirm zero swap/events and test-group removal. Existing
Python (28), exact-fetch JavaScript (13), host (48) and API (47) results are
reused. This is local validation, not authenticated delivery.

The first live comparison (R1) refused before browser launch at local memory
admission. Its 768 MiB child cap could not satisfy the host's unchanged
1 GiB startup-headroom requirement. The child exited and its cgroup was
removed without swap or memory events; no baseline or native result was
obtained. The original `Cleanup` failure is retained. The protected browser
service is unchanged.
The isolated service on port 8788 remains stopped and fenced, with its
original history preserved.

The comparison process owner now supports a 1.5 GiB child inside a 2 GiB
controller, with the host reserve checked separately. A focused local test
confirmed those limits and descendant teardown. Full browser startup and
inference under the larger caps remain unverified. The next live attempt must
also resolve the prior prelaunch fence and honor the reserved cooldown.

The parent wire helper and sequence now pass 25 protocol and nine lifecycle/
history cases. Two real-process tests also confirm graceful rejection and
containment of a stuck descendant in a separate process group. The controller
uses a dedicated bounded cgroup and retains failed or uncertain outcomes.
The pure history helper preserves the original snapshot, reserves both request
slots and extends cooldown through cleanup. These are local tests only.
The first test package omitted a support dependency; a second launcher had a
wrong path. Both failures are retained; the corrected focused run passed.

The standalone parent is implemented and locally tested. The admission/restore
adapter retains account exclusion, resource limits and cooldown publication
through child teardown. The child has no listener or service-management path;
it receives the session only through an inherited private socket after parent
admission. R2 stopped-account adoption and execution remain with the runtime
owner. R1 produced no completed browser baseline or native inference result.

No authenticated browser-token/native-HTTP control was found in the searched
records. The existing browser adapter is not wired into serving and creates a
new browser context. It cannot yet isolate token generation from transport.
Its bearer now uses the browser's validated current cookies, and token age
starts before bearer acquisition. Forty-five focused cases pass, including
cancellation, rejection before authentication and expiry during the bearer
wait. Test compilation peaked at 696,786,944 bytes; execution at 31,268,864
bytes, with zero swap/events and verified teardown. This code is not deployed.
Root owns the missing matched, isolated comparison; the protected browser
remains untouched. See
`reference/analysis/browser-token-transport-control-2026-10-24.md`.

The conversation-ID correction now has a verified staged executable,
`2a0de595`. Its API build peaked at 639,279,104 bytes and final link at
629,932,032 bytes, with zero swap/events and verified teardown. The existing
forty request/authentication/cleanup cases are reused. It is not deployed.

The live-check client's error report now separates valid JSON from completed
HTTP framing. A peer can send valid JSON and close before its declared length;
the report now records that as incomplete. Thirty-five synthetic streaming,
workflow and timing tests pass, including fixed-length, chunked, close-delimited
and capture-limit cases. All non-200 responses still fail. This diagnostic
improvement does not identify or resolve the upstream security refusal.

Current public model helpers close two request questions: GPT-4.1 selects no
provider override, and its model spelling agrees with our native request.
The ordinary browser chooser uses Chat for that model, while the isolated
trial uses Responses. This limits the comparison; it does not establish that
Responses is invalid or caused the security refusal. The joint SDK trace
establishes that token creation starts deferred initialization itself; a
separate warmup call is not the missing prerequisite. A permanently pending
collector triggers the wrapper's deadline/fallback, not a successful V3 token.
Completed V3 output still does not reveal the remote security decision.
No new request or runtime change accompanied this diagnosis. See
`reference/analysis/native-model-contract-2026-10-24.md` and
`reference/analysis/browserless-refusal-diagnosis-2026-10-24.md`.

The current OpenRouter Responses mapper also copies outer `sessionId` into
`request.session_id`. The native request builder now does the same for that
route. Forty focused cases pass, including absent/conflicting nested IDs and
preservation of other routes, payload fields, authentication and cleanup.
The test build peaked at 870,481,920 bytes; tests at 31,047,680 bytes, with zero
swap/events and verified teardown. This change is not deployed or live-tested.
The existing isolated binary remains `3e9398d8` with worker `693051b1`.

The last failed request's saved timing separates 6,063 ms before native send
from approximately 112 ms between send and client-observed 403 headers.
This points toward early request/security rejection; it does not identify
the rejecting server check. Do not treat the six-second total as inference
time. Consolidate remaining request-contract differences before another trial.

The current public AI consumer has now been traced. Its bearer acquisition
includes cookies, while the cross-origin inference request uses the default
credential mode and omits them. A native source correction removes cookies
only from inference, preserving bearer authentication and local session/device
validation. Its test binary compiles and all 39 focused cases pass, including
the retained cookie-bearing login and malformed-credential refusal. Compile
peak was 947,232,768 bytes; tests peaked at 33,083,392 bytes, with zero
swap or memory events and verified teardown. The deployable build passed;
API compilation peaked at 679,120,896 bytes and final link at 572,755,968 bytes.
All 17 update-controller cases passed. Binary `3e9398d8` now runs only in the
isolated instance, retaining worker `693051b1`. Its one live POST made one
upstream send and received a complete 231-byte Framer security 403 after
6.17 seconds to headers. Cleanup and account-history restoration were verified,
with no second request and no change to the browser service. Native lifetime
peak was 277,590,016 bytes with zero sampled swap; this is not a request-only
measurement. The controller exited and its group was absent. Cookie omission
alone did not resolve the refusal. Next, compare remaining current AI metadata
semantics with native construction before selecting another experiment.
See `reference/analysis/current-ai-request-contract-2026-10-24.md`.

Public asset comparison on October 24 confirms wrapper drift: current Framer
selects `chunk-VPFQZMNG.mjs`; the native worker retains `chunk-5LKSDTQ5.mjs`.
The selected V3 SDK bytes and public key still match. The new wrapper changes
token export, dependencies and default V3 timeout, so a filename-only update
is incompatible. The loader now selects its pinned export explicitly.
Seven focused cases pass; the old-loader control retains three expected
failures. The current 14-module candidate passes one synthetic SDK run with
confirmed cleanup at 122,941,440 bytes, zero swap and zero memory events.
Early cancellation and owner-drop checks also pass, without establishing
the SDK phase. The guarded update now selects this wrapper only on the
isolated instance. One POST made one upstream send and received another
complete 231-byte Framer security 403, with 5.24 seconds to headers.
Cleanup, real history restoration and the unchanged browser service were
verified; no second request ran. Native lifetime peak was 140,722,176 bytes
with zero sampled swap. The controller exited and its group was absent.
Updating the wrapper alone did not resolve the refusal. Authenticated
delivery and live interruption remain unverified. Next, examine the saved
browser caller's initialization contract and independent native transport
gaps without replaying this request.
See `reference/analysis/castle-current-selection-2026-10-24.md`.

The native graphics SDK integration now passes with the deployable sources,
peaking at 120,934,400 bytes, with zero swap or memory events and confirmed
worker/group teardown. It generated and discarded a synthetic token.
The instrumented version created five
canvas contexts with no refusal. The bridge now allows two live contexts and
eight total, and preserves native cleanup failures across garbage collection.
A library wrapper had allocated a redundant native context; it now wraps the
existing one. Rendering and eight canvas checks pass, including exact native
counts, discarded-canvas collection, live-canvas preservation and owner refusal.
The worker keeps graphics diagnostics outside its strict metadata protocol.
Earlier memory and setup failures remain recorded in the new qualification.
The guarded update is now active only on the isolated instance. Its one live
POST made one upstream send and still received the complete 231-byte Framer
security 403 after 5.42 seconds to headers. Cleanup and history restoration
were confirmed, with no second request. The browser service stayed unchanged.
Native lifetime peak was 179,019,776 bytes with zero sampled swap. This does
not establish successful streaming. The next independent check is cancellation
of the graphics-enabled worker without a live request.

A genuine Mesa WebGL implementation now passes rendering, native-identity and
cleanup checks at 93,655,040 bytes. Its bounded HTML canvas bridge passes six
checks with real JSDOM/canvas at 103,636,992 bytes. Both retain the 128 MiB cap,
zero swap/events and verified teardown. Collection before native context
allocation resolves the tested combined-memory failure. Earlier failures and
standalone native-query measurements remain preserved. SDK integration is the
next step; no live request or service change followed from these component tests.
See `scripts/experiments/native_gl` for the exact patch and saved-result bindings.

A new bundle fills the SDK's separate dedicated Worker API gap. Its
two pinned scripts now run in real Node threads with the native user agent.
Eight focused cases pass, including active cancellation and private errors.
The instrumented SDK run received the expected feature, entropy and timing
replies and joined both workers. The deployable bundle also passes its eight
cases and a synthetic SDK mint, peaking at 102,232,064 bytes with zero swap
or memory events and verified teardown. The isolated instance now selects it.
One live request made one upstream send and received the same complete Framer
security 403 after 3.80 seconds to headers. Cleanup, restored history and the
unchanged browser service were verified; no second request ran. Native lifetime
peak was 122,404,864 bytes with zero sampled swap. These describe a refused
request, not successful delivery or interruption. The next investigation is
whether genuine native graphics can fill the remaining WebGL API gap without
launching a browser. No further live request is justified by this result alone.

A cookie-alignment candidate now carries the admitted device into the bearer
request as well as the later inference request. The isolated stored login has
only the session cookie; the previous native source added the device after
bearer acquisition. The candidate derives a temporary request context, keeps
the saved login unchanged, and rejects a joined mint that returns a different
or missing declared device. Authentication failure still prevents minting.
All 17 focused source/authorization cases pass, including conflict-before-work
and changed-device-after-join cases. Compilation peaked at 847,159,296 bytes;
tests peaked at 32,190,464 bytes, with zero swap/memory events and clean teardown.
The full application build also passes: API peak 574,832,640 bytes and final
link peak 429,305,856 bytes, with zero swap/events and clean teardown. Binary
`c358abfc` now runs only in the isolated instance, with its real history restored.
One live request made one upstream send and received a complete Framer security
403 after 4.18 seconds to headers. Cleanup and final pool idle were confirmed;
no second request ran and the browser service remained unchanged. The corrected
log binding observed this result directly. Aligning the cookies did not resolve
the refusal. This generation peaked at 135,573,504 bytes with zero sampled swap;
these are failed-request measurements, not successful delivery measurements.

A source-tested diagnostic now records whether the login-token response
contains cookie updates. The existing control transport discards those headers;
the saved live evidence cannot say whether any were returned. The diagnostic
emits only HTTP status and bounded counts for session, device, other and malformed
cookie headers. It never records values or unknown cookie names and does not
change cookie handling. All 20 focused authorization and privacy cases pass.
The application build passes with API peak 570,425,344 bytes and link peak
409,485,312 bytes, zero swap/events and clean teardown. Binary `d950da60` is
now active only in the isolated instance. One diagnostic request obtained its
login token with HTTP 200 and no Set-Cookie headers, then received the same
complete security 403 from inference after 3.70 seconds to headers. Cleanup
and pool idle were confirmed, and no second request ran. The browser service
remained unchanged. Discarded login-response cookies do not explain this attempt.
The supplied agent category, feature name and step match the browser rules;
the client derives the prompt and fresh turn/session IDs for each request.
One separate bounded dashboard GET returned HTTP 200 and the original selected
project exactly once in its project list. Project visibility is established;
AI entitlement and the reason for the security rejection remain unresolved.

A new offline framing probe also excludes a prefix-only result in one synthetic
SDK run. The unchanged SDK produced a complete 3,951-byte frame with 36 clock
bytes and 3,913 protected payload bytes. Four parser cases pass. The existing
owner joined the worker and discarded its token; group peak was 86,765,568
bytes with zero swap/events and verified teardown. Production code is unchanged.
This does not inspect the token from a rejected live request. The source trace
identifies the SDK's empty-frame overflow as a clock-metadata limit, not a
browser-feature-size limit. See `scripts/experiments/native_envelope`.

The deployable graphics worker is now integrated in browserless source and
validated in a separate pinned bundle. All 34 component cases and a synthetic
mint through the existing owner pass. The mint peaks at 84516864 bytes with
zero swap/memory events, discarded token, joined worker and absent unit/group.
The fixture-only output is removed. A per-window quota retains the bounded PNG
policy; external/file/other data resources stay disabled. No Rust rebuild is
needed because the current application accepts explicit worker configuration.
The isolated instance now selects this bundle. Its real history was transferred
and restoration verified, without changing the application binary or routing.
One live request made one upstream send and received the complete HTTP 403
security envelope after 3.56 seconds to client headers. Cleanup was confirmed;
there was no second request or admission refusal. The protected browser service
remained unchanged. The isolated service's lifetime peak was 93,794,304 bytes,
with zero sampled swap; this is not successful-delivery memory or latency.
Canvas support alone did not resolve this refusal. The consumed request will
not be replayed.

The next worker bundle now supplies a real SharedWorker exchange using a Node
worker thread and native message ports. It admits only the pinned SDK script,
retains the native user agent, leaves browser-only metadata absent, and joins
the thread before handing off the token. Eight focused cases pass, including
cancellation while the thread is executing JavaScript. The unchanged SDK
returned its expected worker reply in a separate instrumented check. A mint
with the deployable sources also passed, peaking at 85,839,872 bytes with zero
swap/memory events and verified teardown. This bundle is now selected only by
the isolated instance. Its first request made one upstream send and received
another complete HTTP 403 security envelope, after 3.69 seconds to headers.
No second request ran. Adding SharedWorker support did not resolve the refusal.

The client reported a cleanup timeout because its copied plan still named the
previous day's log after the application restarted after midnight. The correct
saved log contains one send and confirmed native stream cleanup. That recovers
the missing stream evidence, but does not retroactively pass the client or
prove final pool quiescence. The original failed report remains intact.

The client now checks that the planned log is open for append by the exact
application process before sending, and rechecks the descriptor while reading.
Seventeen focused cases pass, including a stale daily path refusing before
any request, a valid older log remaining usable, and changed-writer refusal.
The isolated test group peaked at 17,084,416 bytes with zero swap/memory events
and verified teardown. This client correction is source-tested, not activated
by another live attempt. The next execution plan must bind the actual writer's
log and use the corrected client; neither the date nor an inherited filename
is sufficient. Further live work still requires a concrete changed hypothesis.

The remaining process warning is now identified as Node's experimental
VM-module notice, originating in our SDK loader. The diagnostic retained only
counts, and found no other process warning. It supplies no fix for the 403.

The official Castle documentation has now been checked against our worker.
Its Browser SDK requires a browser, and its 3.0 migration guide explicitly
removes support for mocked DOMs such as JSDOM. Our local worker uses JSDOM;
its successful token generation therefore does not establish a supported
runtime or Framer acceptance. Castle's Node server SDK consumes frontend
tokens, while the native client SDKs target mobile platforms. This does not
prove the specific cause of the saved 403. See
[runtime support](castle-runtime-support.md) for the official sources.
The reviewed Framer Server API and Plugin API documentation did not establish
a native AI completion/token method. Framer MCP uses the connected agent's
plan or credits, so it does not establish the requested Framer-backed path.
The SDK's outer V3 selection and nonempty return do not certify its internal
token kind. The completed static trace shows that individual missing-feature
checks can resolve diagnostic fields inside the normal output. Escaping
exceptions still reject the current operation. No separate fallback/error
token is established for the live refusal. The JSDOM limitation was already
recorded on October 20. No SDK or live request was repeated for this trace.

A separate native canvas fixture passes real pixel drawing/readback, PNG
round-trip, resource refusals and context-type checks. The unpatched first
run failed because JSDOM returned cached 2D for WebGL. A four-line dependency
patch corrects that behavior. The second run peaked at 73601024 bytes/ten
tasks under a 128 MiB group limit, with zero swap/events and verified teardown.
Its published source is under scripts/experiments/native_canvas. A subsequent
synthetic SDK-worker integration passes using the existing JITless owner.
The first integration minted but failed the resource check at the unchanged
128 MiB cap. Applying the existing 64 KiB verification-buffer implementation
to the larger native-library inputs yields a passing second run: 84398080
bytes group peak, 12 tasks, zero memory events/swap, two local PNGs and zero
JSDOM errors. The process warning is classified above. The unchanged private
token pipe, device checks, joined worker, discarded token and final unit/group
absence all pass. No Rust build, account action or Framer request ran.
The later deployable bundle omits experimental diagnostic output and is selected
only by the isolated instance, as recorded above. Unsupported JSDOM remains a
documented limitation; the local success did not resolve the live 403.

The enrollment build is running in its separate loopback instance on port
8788. A legitimate independent login and its observed device have been imported
and enrolled successfully, with the initial full 600-second cooldown enforced.
The browser service and its account set have not been changed.

The streaming-refusal correction first ran in that isolated instance
as binary `be15778f`. Its old generation's real device history was captured
before its processes were stopped. The replacement restored that history
through a one-use handoff and enforced a full additional device cooldown.
One live request made one upstream send and received HTTP 403 with a complete
Framer security error envelope. Cleanup was confirmed and the pool returned
to idle. No second request ran. The protected browser service is unchanged.

The next compatibility correction adds `Referer: https://framer.com/` to
native inference HTTP. It sends only the public application origin, without
exposing or inventing a project path or query. The real loopback fixture now
checks it together with Origin, credentials and the truthful native user
agent. All 14 focused transport, stream and cleanup cases pass, with zero
recorded swap or resource events and clean teardown. Test compilation peaked
at 544,919,552 bytes; the test run peaked at 43,532,288 bytes. The complete
application rebuild also passed. Its native API phase peaked at 558,170,112
bytes and the final link at 412,545,024 bytes, with zero recorded swap or
resource events and clean teardown. Candidate `464b413d` is now active only
in the isolated instance. Its real history was restored without changing
account identities, devices or the current account records. One new request
made one upstream send and again received a complete HTTP 403 security
envelope, after 2.63 seconds. Cleanup was confirmed and no second request
ran. The browser service remained unchanged. This rules out the header
correction alone as sufficient for that attempt; authenticated delivery and
live interruption are still unverified.

The preceding Origin candidate restored its real history successfully. One request
made one native send, then returned a 60-second deadline error without text.
Cleanup was still unconfirmed after the following 30-second observation;
the second request did not run. This is not successful delivery or evidence
that the earlier 403 is resolved.

A source-level deadlock is now fixed: a composed refusal's error body owns
the same memory reservation that cleanup waits to recover. The outer router
used to retain that error while awaiting cleanup, preventing both from
finishing. Streaming calls now transfer that body to the caller first and
retain the account/device owner until actual cleanup. Collected JSON calls
preserve their original cleanup-before-error ordering. Failed HTTP heads also carry the
existing independent cleanup observer, including when their body is dropped.
Seven forwarding cases and 23 server stream/ownership cases pass. The final
streaming-only refinement has three fresh passing cases, including collected
JSON ordering, and reuses four unchanged forwarding and all 23 server cases.
The fresh run peaked at 36,446,208 bytes, with zero recorded swap or resource
events and clean teardown. The first complete application build passed; the
final refinement also linked successfully, peaking at 402,513,920 bytes with
zero recorded swap or resource events. It is deployed only to the isolated
instance. The old timed-out request is not replayed.

The live client now supplies a fresh turn ID and matching prompt for each
request. Its interruption waits for both the actual native send and HTTP 200
stream headers, before this adapter renders its buffered response. It still
requires confirmed cleanup, exactly one upstream attempt and successful
headers before admitting the second request. The focused workflow cases
pass; the remaining unchanged client cases reuse their saved results.
The first local request stopped with HTTP 502 before any native inference
send. No second request ran. The example's 64 KiB composed-state allowance
is too small: fixed reservations alone require 66,048 bytes before payloads.
The example now allows a bounded 4 MiB without changing process or worker caps.
Six focused Rust cases pass against the deployed libraries, including the
original-budget refusal and corrected-example authorization boundary.
The client now retains only allowlisted native error codes and saves its
failed stream snapshot. All 29 focused streaming/workflow/timing cases pass.
A separate bounded nonstreaming request reached native HTTP and received
HTTP 403. It returned the isolated account to idle, with zero recorded swap.
Its 2.61-second local interval includes preparation and does not identify the
remote refusal mechanism. The original response category was not retained.
Both local POSTs are consumed; neither is replayed automatically.

The native transport now supplies the canonical Framer application Origin.
It keeps its truthful native user agent, unchanged credentials and existing
single-send behavior. A real loopback request verifies the header; all 14
focused transport, streaming and cleanup cases pass. This closes a concrete
request-construction omission, not an established cause of the live 403.
The complete application rebuild passed and its candidate is staged. The
API phase peaked at 877,047,808 bytes and the final link at 592,891,904 bytes,
with zero recorded swap or resource events and verified teardown. Isolated
activation must preserve actual device history; the browser service stays
unchanged. Authenticated delivery and live interruption remain unverified.

The failed-response checker now records only bounded content categories,
JSON validity/size and whether a complete known security-error envelope was
present. It never retains response text, reference IDs or timestamps. All
32 focused streaming, workflow and timing cases pass, including the three
new diagnostic cases; their saved inputs match the published source.

A separate GitHub Actions workflow builds the native candidate with pinned
Rust 1.99.0 and the locked dependencies, and publishes an executable with its
source commit, compiler identity and hashes. This moves compilation away
from the running services. Its first run was refused before any step because
GitHub reported failed account payments or a spending limit. No remote
compiler ran. The first local build admission also refused insufficient
memory reserve. A later admission met the unchanged requirement and the
local rebuild completed. Both earlier refusals remain preserved.

The isolated transfer controller has nine passing focused fixture checks.
It retains a private, genuine cooldown/device snapshot, requires the old
process group to be absent before replacement, preserves the launch sandbox
and checks the new service invocation. Its original binary and routing are
backed up before any stop. The first isolated replacement started but its
host could not inspect the outer controller across the sandbox's user
namespace. That request failed before any upstream send. A second controller
placement failed its full guard before arming a handoff or sending a request.
Both failures and the original account history remain preserved.

Five additional recovery cases reject snapshot reuse after any native send,
restoration, changed account state or additional operation owner. Real inert
namespace probes reproduce the permission boundary. The final arrangement
keeps full service checks outside and an identity witness inside the native
user namespace. Its fixture confirms both checks and clean child exits.
The isolated generation was reused for the bounded request described above.
That controller correction needed no further application build or restart.
The browser service remains unchanged.

Earlier build and preparation milestones below retain their original limits.

Initial native enrollment is now implemented in source. The new native-only
account endpoint checks the expected login, stored session and exclusive
account ownership before asking the host to enroll the observed device.
The host requires explicit startup opt-in, refuses prior account/profile
history, and charges a full cooldown/window before any first request.
Enrollment does not manufacture a restoration receipt or clear old history.

All 34 focused Python host cases and 31 focused Rust server cases pass,
including nine new cases in each layer. The Rust selection retains the
affected account-management and import coverage. Both runs use synthetic
state, with zero swap/events and verified teardown. The Rust run peaked at
17,940,480 bytes; the Python run peaked at 34,918,400 bytes.

The compatible core, host, API, store and server libraries and full application
now build successfully. Candidate `afb3c82e` contains enrollment and is staged
with the explicit generated host `161b3206`. Its 23 worker configuration pins
and 1,247 dependency entries were verified. This is a staged candidate, not
authenticated inference or a changed browser service. Independent initial
enrollment and the bounded real streaming sequence are next.

The retained JITless cancellation, device-bridge results and memory-attribution
reviews are now complete. All three accept source and saved evidence with no
findings. Their exact receipts are bound to the existing inactive preparation;
no validation was repeated and no running binary changed.

A bounded read-only check found an authenticated saved Framer login outside
the browser service's current configured account set. The browser generation
remained unchanged. No cookie was exported or account imported. Native
admission still requires genuine device selection and cooldown history;
validation and owned initial admission are the next integration tasks. This is
not authenticated inference or permission to manufacture restored state.

An account-import candidate now checks an optional expected Framer
identity before saving the session or provisioning workspaces. It also skips
automatic browser prewarm when native dispatch is configured. All eight
focused import cases pass, including four new identity/prewarm cases. The
accepted application executable and service configurations remain unchanged;
separate authentication and actual device/cooldown authority are still required
for a live browserless request.

Import validation first stopped before compilation because its launcher and
runner disagreed on the test-unit name. After that setup correction, compilation
found a missing optional identity field in one existing test initializer. The
initializer is corrected. The fresh R3 test build and all eight selected cases
pass; summary `8f46157b` binds both results and the retained failures. Compilation
peaked at 705,204,224 bytes and testing at 17,985,536 bytes, with zero swap and
verified cleanup. These are validation-process measurements, not service memory.
The corrected server library and full application link also pass. Candidate
`1baa4a98` is built but has never been launched or deployed. Their compilation
peaks were 377,405,440 and 530,116,608 bytes, with zero swap and verified cleanup.
Summary `7fff81b6` and freeze `e50f1424` bind the source, eight cases, both builds
and retained failures. Review `7225960e` accepts the exact source and saved
evidence with no findings; it grants no runtime or account approval. Preparation
R6 selects this candidate, retains the ten other runtime entries and is still
non-executable: account, generation and concrete client plan remain unbound.
The candidate still requires explicit selection of the accepted lazy host.

Browserless is not complete: the isolated setup has no verified authenticated
Framer delivery. Import and initial enrollment now succeed for the separate
login. The bounded live check must still demonstrate delivery and cleanup
after the full initial cooldown; source and synthetic tests cannot prove them.

The deferred IDNA-table experiment is complete and remains unapplied. Original
and candidate outputs match across 105,216 calls each, but one synthetic mint
showed no useful peak-memory improvement. Summary `8cb638bf` retains the exact
comparison and cleanup evidence. The probe and reproduction patch are published;
no dependency, application binary or running service was changed by this study.

The experimental owner source now selects `--jitless`. Its five files match
the already built and measured variant exactly. Owner/host review `df13c06d`
accepts that source and saved integration evidence. Both cancellation cases
also pass; review `a7aae40d` accepts their source and saved results. The targeted
application link now passes: result `05552129` binds executable `8ffa3077`
and 6,272 inputs. The binary has run only in isolated synthetic fixtures;
it is not deployed. Running services and browser-based branches are unchanged.

A combined application, lazy-host and separate owned-worker measurement now
passes. Its group peak is 89,899,008 bytes, while sampled process PSS with
explicit RSS fallback peaks at 159,323,136 bytes. Summed process RSS peaks at
195,010,560 bytes. These are different accounting measures, not interchangeable
estimates. The result does not establish the 100 MB service target.
The synthetic worker completed in a 2,233.986 ms owner interval. It joined,
the application remained alive afterward, and final cleanup left no unit or
cgroup, with zero swap/events. Eight new sampler cases pass; summary `9e644000`
binds those results and the unchanged probe/application binaries. The failed
first measurement and prelaunch directory-setup error are preserved. No
authenticated inference, browser-service change or live-account access occurred.
Review `85f3d29c` accepts this frozen source and saved measurement, with no
findings. It grants neither runtime approval nor acceptance of the 100 MB target.

A later attribution sample separates known measurement helpers from the
application, host, worker and separate owner probe. Their simultaneous PSS
total is 142,124,032 bytes: worker 92,988,416, application 25,802,752, host
20,989,952 and owner probe 2,342,912. No unknown role occurred at that peak.
This still exceeds the 100 MB target and includes a standalone owner probe,
not the fully integrated request path. It is attribution, not a saving.
Fifteen focused sampler/role cases pass; summary `42afcec1` binds the new
diagnostic and exact source. The same binaries ran under unchanged limits,
with zero swap/events, joined processes and no remaining unit or cgroup.
Review `450e8928` accepts this attribution source and saved evidence. The
16,559,104-byte helper sample remains separate from the 142,124,032-byte
non-helper sample and from process-group accounting.
The next memory investigation targets the worker's allocation phases;
authenticated delivery continues to require the separate account.

The current-flag nine-phase worker probe now passes. JSDOM loading added
21,561,344 bytes of anonymous memory. A separate pre-mint collection
experiment also passed its original device and cleanup checks, but its
158.083 ms collection yielded only about 1 MB less reported peak RSS in
one comparison and 91.081 ms more total minter time. It remains unapplied;
no reliable service-memory or latency improvement is claimed. Summary
`72049ab5` preserves both runs, and the reproduction patch is under
`reference/analysis/offline-worker-pre-mint-gc.patch`. Both workers joined,
units/cgroups disappeared and swap/memory events stayed zero. No compiler,
account, running-service or authenticated request action was involved.

Review `4e69d24b` accepts the application source and saved build bindings with
no findings. It grants no runtime or delivery approval. Preparation `398c1c63`
selects this binary while retaining the ten other runtime declarations and
the existing client. Account, generation and execution plan remain unset.

The successful build peaked at 725,790,720 bytes under its unchanged 768 MiB
cap, with 12 tasks, zero swap/events and no remaining build unit or cgroup.
An earlier memory-cap failure and a diagnostic-file ordering refusal are
preserved. The wrapper now releases unused verifier allocations and writes
its memory diagnostic after the empty-directory check. Reclaim was about
10 MB, so it does not establish the first attempt's high-memory cause.

Live readiness checked at 19:12 UTC on 2026-10-23: the isolated store has
zero accounts and zero saved sessions. Its earlier native test instance ended
at 16:37:56 UTC under its configured 30-minute limit. No authenticated live
request has been sent from this isolated setup. The accepted executable stays
staged. A legitimate separate account, session and device/cooldown history are
required before the bounded live check. These read-only checks did not access
the browser service or change either service. Local tests do not establish
successful authenticated delivery.

Six isolated worker-memory experiments are complete. Five synthetic local
SDK mints passed their existing device and cleanup checks; the 32 MiB heap
trial exited with code 134 before producing a result. Neither heap flags nor
the successful reduced heaps demonstrated a useful saving. Production keeps
its accepted 96 MiB setting. All test groups are gone, with zero swap/events.
The new probe and [measurement report](offline-worker-memory.md) preserve
these results, including the failure and limits of the RSS/group comparison.
Final review `c87b7653` accepts the original source and six saved runs with no
findings. Its scope excludes the subsequent allocation and cache study.
Real streaming still awaits a separately owned account;
the accepted executable and timing client remain prepared but not activated.

The subsequent allocation probe identifies JSDOM loading as the largest early
increase: 25,042,944 bytes of additional anonymous memory before window creation.
Three static-cache comparisons averaged 4.1% less local minting time and 5.0%
more peak group memory. Production caching remains disabled and the heap stays
at 96 MiB. All completed study inputs, outputs and teardown records are checked
in local summary `5a1b30b6`; these later results are not independently reviewed.
One subsequent run tested a 64 KiB reusable verification buffer while retaining
every digest check and the existing DOM/device environment. The local mint
passed, but its 75,436,032-byte group peak and 1209.069 ms elapsed time did not
improve on the saved baseline means. That experimental patch remains unapplied.
An import-only probe then observed 638 uncached module loads. CSS style
prototype setup had the largest individual net heap increase, about 6.97 MB.
The process group peaked at 41,947,136 bytes with zero swap/events and clean
teardown. This excludes SDK execution and is not a full-worker improvement.
Summary `89f3ecc3` and the published probe preserve the diagnostic and its
GC/nested-allocation limits. No service, dependency or account was changed.
The prototype-construction experiment then matched all 1,553 descriptors and
1,537 per-setter observation hashes. Its import-and-fixture group peak fell
from 49,573,888 to 43,384,832 bytes. A full synthetic mint with the same
dependency copy passed, but its 74,878,976-byte group peak was essentially
the existing baseline range, and its 1330.950 ms elapsed time was higher.
That patch also remains unapplied. Summaries `5a262d7f` and `c46984c2` preserve
the results and limits; they are not independently reviewed.
The subsequent actual-owner measurement passed using the unchanged library
from the accepted application build and the original worker entry. Its real
private-pipe, metadata and freshness checks completed; the worker joined and
was absent afterward, and the synthetic token was discarded and cleared.
Peak group memory was 75,116,544 bytes with 12 tasks and zero swap/events.
Owner elapsed time was 1978.228 ms, including startup, minting, handoff and
join. The unit and cgroup disappeared. This single synthetic sample excludes
account/host admission, HTTP, authenticated delivery and total service memory.
The exact harness and summary `c3edd067` preserve the passing build and run,
two compiler-wrapper failures and one prelaunch resource refusal.
Final review `089c5975` accepts frozen package `52d1ae7e` for source and saved
evidence, with no findings. No repeat build or measurement is required.
Production is unchanged, and the separately owned account remains pending.

A later JITless synthetic sample reduced group peak to 57,331,712 bytes
and reported process RSS peak to 96,497,664 bytes. Compared with the three
saved baseline means, these are reductions of 23.58% and 21.41%; minter time
increased 7.13% to 1271.386 ms. All original device and cleanup checks passed,
with zero swap/events, joined processes and absent unit/cgroup. This one
sample excludes the Rust owner, host and authenticated inference. Summary
`841d7c33` preserves the result without rerunning a baseline. The measurement
tool permits this flag for further controlled comparison; production remains
unchanged; this sample is included in the subsequent review below.

Two further JITless samples passed using the unchanged worker and guards.
The three-sample mean is now 57,839,616 bytes of group peak memory and
96,922,283 bytes of reported RSS, respectively 22.90% and 21.07% below the
three reused baseline samples. Mean local minting time is 1230.173 ms,
3.65% higher. All processes joined, units/cgroups disappeared and swap/events
stayed zero. Summary `b55a2a79` preserves the original and two new results.
This supports the memory direction across three small samples; it does not
establish service memory or request latency. Production remains unchanged.
Review `f68c813b` accepts the three-sample source and saved evidence without
findings. A subsequent isolated owner variant now has two passing builds and
two passing integration measurements. Actual-owner group peak was 59,097,088
bytes versus the saved 75,116,544-byte baseline. With the host and its device
lease, peak was 82,403,328 versus 102,887,424 bytes. Both cases retained joined
cleanup, zero swap/events and absent units/cgroups; the host released its
lease and admitted a fresh one in the same generation. These are single
synthetic samples, not HTTP or authenticated delivery. Summary `dd8002d2`
binds the new results and reused baselines; review `df13c06d` accepts its
source and saved evidence without findings.
The same variant also passes the existing explicit-cancel and dropped-owner
cases after observed worker activity. Both join the worker and leave no unit
or cgroup, with zero swap/events. Summary `6a451af4` binds their separate
harness build and two runs. The SDK phase was unobserved, and these cases
do not establish HTTP-stream cleanup. The experimental owner source now
matches the measured variant. The application now links that verified owner
library. Review `a7aae40d` accepts the cancellation evidence; application review
`4e69d24b` accepts the source and saved build bindings. Real account readiness
and authenticated delivery remain unproven.

Two additional local cases now cancel the real worker after observed process
activity. Explicit cancellation returns the expected failure; dropping the
owner also joins the child. Both leave no worker, unit or cgroup, with zero
swap/events. Their 3.268 ms and 2.896 ms interruption-to-join samples do not
establish real-stream cleanup or user latency. The SDK phase is unobserved,
and their lower memory peaks reflect early termination. Summary `ba8d6ac0`
and the exact tested harness are accepted by final review `72ba6876`, with
no findings. This accepts source and saved evidence only.

One subsequent combined measurement held a real native host device lease
while the unchanged owner and worker completed a synthetic mint. The host
kept the same generation, confirmed joined completion and released the lease.
A fresh lease then opened and closed without another mint. The group peaked
at 102,887,424 bytes with 15 tasks, zero swap/events and no remaining unit or
cgroup. Owner elapsed was 1934.152 ms. This includes the host, worker and
measurement helpers, but no application server or authenticated inference.
The published Python harness and saved summary `6af8688a` preserve the
result. Final review `d98b96cf` accepts their source and saved evidence with
no findings. No production source or service changed.

Three additional tests of the production caller now pass. They cover
explicit cancellation, dropping the result waiter and losing the host's
completion acknowledgement. The first two retain account closure while the
host reply is withheld; the third requires unconfirmed cleanup and refuses
fresh preparation or reopening the same account. A real worker process and
account preparation are used with a controlled host reply and synthetic input.
No SDK or network request is involved. The earlier three prelaunch memory
refusals remain preserved. Both small harnesses now compile, and the candidate
passes all three cases. The deliberately broken version fails the expected
host-acknowledgement assertion. Summary `167fb923` binds these results.
The runtime groups peaked at 19,230,720 and 19,218,432 bytes, with 12 tasks,
zero swap/events and no remaining units or cgroups. These tests retain the
original owner-library binding; they do not validate JITless HTTP behavior.
Source-only review `080d4e9a` found one preparation blocker: all three allowed
build labels were consumed by the saved prelaunch refusals. An additive
correction uses a separate output namespace while retaining three labels per
mode, exclusive creation and every resource guard. It also binds the exact
launcher, dependency manifest, label and mode when selecting build results.
Review rejected freeze `4745f620`: the unchanged compiler entry still required
the original output parent. R3 freeze `8b103a0a` keeps that parent and uses
fresh bounded attempt labels 4 through 6. Its static checks cover the actual
entry assertion and detect the R2 mismatch. Review `5d6b6cd0` accepts R3's
source and static bindings, closing both findings. The subsequent builds
and runtime cases passed fresh resource admission without changing those
guards. Review `613d33ce` accepts the two builds, three passing cases and
the intended failing control. Its acceptance covers saved evidence only.
Both prior rejections remain preserved.

The isolated client now records monotonic request timings, starting before
connection/request work. It distinguishes headers, first complete text,
terminal completion, socket disconnection and reader exit. Five new socket
and caller tests pass with 22 existing stream/workflow/runner cases. They cover
heartbeat and partial-frame exclusion, retained first-text time, terminal text,
wall-clock changes, parse failure and actual cancellation. The isolated run
peaked at 18,374,656 bytes with four tasks, zero swap/events and joined threads;
its unit and cgroup are absent. This Python-only change does not rebuild the
staged server or establish live latency. Final review `d9760e73` accepts the
source and saved evidence without findings, binding freeze `2a4ca7ad` and
summary `3eab1e9f`. Actual prelaunch memory and disk values were not serialized;
the saved caps and passing run do not establish current resource admission.
The isolated preparation now binds all ten accepted client files. Account,
generation and execution plan remain unset. Next: bind the separately owned
account and freshly admit the isolated stream check. The accepted executable
remains staged; the browser service and its account scope remain protected.

Native send accounting now covers attempts that fail before response headers.
The separate checker requires both the send events and the cumulative count
after original stream cleanup. This supplements the existing response metadata
count, which could miss an additional failed send. All 25 affected Python cases
and 13 selected Rust cases pass, including three new transport cases. The old
checker fails the hidden-send regression. Original failed candidate/validator
results are preserved. The API library and final executable builds also pass;
binary `b4151ab3` is staged. The unchanged host library still embeds the earlier
default, so the accepted lazy host requires explicit sidecar selection. This
change has not been deployed. Final review `baef93ed` accepts its frozen
source and saved evidence with no findings, binding freeze `d1bdcf10` and
summary `31f9210e`. The old-checker negative control uses current fixture
support. This acceptance does not establish a live account or delivered stream.

The native host now defers Camoufox, Playwright and optional browser memory
setup until a browser launch. Five focused tests pass; the native-startup
regression fails against the prior source. The three existing real-server
restoration cases also pass with both browser libraries hidden. The generated
candidate host is `e09123fe`; its support package is unchanged. The server
binary is reused, and no service or account state has been changed.

Matched isolated host-initialization measurements recorded resident memory
of 91,348,992 bytes before and 26,550,272 bytes after this change, a reduction
of 64,798,720 bytes. The candidate loaded no Camoufox or Playwright modules;
the baseline loaded 78. These are one-process, one-sample measurements before
SDK work or inference. They do not establish total service memory or latency.
The three server cases peaked at 41,328,640 bytes, with seven tasks, zero swap
and no memory events. Their processes joined and the test unit/cgroup are gone.
Final review `35d0b188` accepts the source and saved evidence with no findings,
binding freeze `1e7ef8bc` and summary `a36c1078`. Verification `1eb6de8f`
covers all 106 seals. This does not accept an actual browser launch, SDK run,
authenticated stream or current deployment. Real-account validation remains
pending, and the live browser service remains untouched.

The current instruction is to leave the live browser service untouched and
run browserless in a separate instance. T3 update work is set aside. The live
browser's binary, launch files, accounts and retained maintenance marker must
not be changed by the parallel experiment.

Journal evidence identifies two startup failures as `saved account state
changed`. The generated host previously selected its cooldown handoff from
the script directory, even when launched with another profile root. The new
source selects the instance root from its canonical profile path and adds an
explicit `--sidecar-script` option. Five isolated Python fixtures pass,
including the original shared-directory failure and continued rejection of
changed account snapshots. Both Rust host-selection cases and the host/main
build pass. The process fixture also passes: an actual generated host ignores
a malformed handoff beside its code, and the isolated API rejects malformed
JSON and an unknown account without crashing. This fixture uses an empty store
and a private network namespace; it does not contact an upstream model.

At 2026-10-23 15:51 UTC the separately launched instance on port 8788 passed
health, empty-account/pool and HEAD checks. The browser service retained its
original process generation. The new binary is active; its prepared host and
worker configuration are staged, with no host, worker or browser child running.
The instance has its own writable root and masks the browser account paths.
Its 5,378,048-byte peak and zero swap describe only this empty-instance sample.
It has no automatic restart and a 30-minute maximum lifetime.

Independent review accepted the frozen source and saved evidence with no
findings (receipt `a679d7f8`, freeze `c0d7a7a3`, validation `2c22b87d`). This
acceptance covers the startup fix and recorded checks, not live inference.

At 2026-10-23 16:07 UTC a guarded switch replaced only the owned empty test
instance. `framerize-browserless-native-r1.service` served port 8788 with
the validated native routing configuration. The browser service kept its
process generation. A normal streaming request reached native dispatch and
returned `503/Ineligible` before any worker started. Activation receipt
`6a429355` records a 5,238,784-byte empty-instance peak and zero swap/events.
The instance retains its 30-minute limit and has no automatic restart.
That saved observation does not establish current availability.

Next: a separately owned account for real browserless validation. A read-only
inventory found that the browser service's saved policy includes all 11 saved
accounts, so none can simply be copied into the parallel instance. Account
selection is pending. Empty startup and routing results establish neither
authenticated delivery nor memory use under inference load.

The next source change adds `native.stream.complete` at the existing joined
native completion/cleanup observer. It records only bounded outcome, status,
attempt-metadata count, yielded bytes and timing fields. It keeps body failure
separate from cleanup confirmation. The active binary does not contain this
change yet. Three focused cases for observer ordering, cleanup failure and
exclusion of provider metadata pass alongside all 22 selected ownership
regressions. The server library and final executable also build successfully
under the existing caps. Binary `a8b5bf6e` is staged, not installed. Independent
review `14eddc0d` accepts the source and saved results with no findings.
This event supplements the real-stream check;
it does not prove delivery or count every transport start by itself.

The separate native stream checker initially passed 38 synthetic fixtures. It
requires a disjoint account, a pinned isolated generation, actual streamed
text before interruption, confirmed cleanup and a subsequent complete stream.
The fixture run used 19,009,536 bytes at peak, four tasks and zero swap or
memory events. All reader threads joined; the test unit and cgroup are gone.
No live checker invocation or authenticated browserless request has occurred.
The account selection remains pending.

Client review found one missing guard: a queued resident close can have zero
users before its close operation starts. The checker now requires explicit
idle status and no replacement, in addition to the original checks. Twelve
workflow and runner cases pass, including new regressions at both request
barriers; 28 unchanged case results are reused. This covers 40 distinct cases.
The new run peaked at 18,964,480 bytes with three tasks and zero swap/events.
The original rejection and results remain preserved. Final review `26089a3b`
accepts the correction's source and saved evidence, closes F1 and reports no
findings. It binds freeze `075b12d9` and summary `13bde1a3`. The accepted
binary remains staged; no account, execution plan or live result is covered.
No live request or service action has occurred.

Six additional process cases now pass against the unchanged generated host
and support package. They cover nonempty synthetic restoration, real selected
device and rate-history admission, retained mint charges, uncertain ownership,
changed-account JSON and replay refusal. This closes the gap between earlier
in-process restoration fixtures and empty-store process checks. The fixture
peaked at 123,883,520 bytes with seven tasks and zero swap/events. All nine
fixture processes joined, and the temporary unit and cgroup are absent.
Neither running service was queried or changed. No worker SDK or inference
ran; the separate-account dependency and staged-binary status remain.

Final restored-host review `b0868885` accepts source and saved evidence with
no findings, binding freeze `74c02edc`, summary `987ac63c` and publication
`c6c49e2b`. Verification `acf906aa` covers 26 seals, five published files,
49 synthetic outputs and the six saved cases. Its scope is the generated
host with a Python fixture parent; mint completion and cleanup replies are
injected. Process joins do not prove application cleanup. No daemon recovery,
authenticated inference or current service availability is accepted. The
review receipt was verified without rerunning tests or querying either service.

Three additional process cases now exercise the built server and generated
host together using complete synthetic account records. Two missing-session
requests on one generation return `502/Authorization`, release the device and
leave the pool idle; hello HEAD remains usable. A restored full window returns
`503/Ineligible`. Changed account JSON returns `502/Cleanup` without publishing
restoration. The original fixture expected 503 for that last case; its failed
result is retained. Only that assertion was corrected and that case rerun;
the other two passing results are reused. No production bytes changed.
The two runs peaked at 127,746,048 and 109,379,584 bytes, with 10 and 9 tasks.
Current and peak swap were zero; all owned processes joined and both temporary
units and cgroups disappeared. Real accounts, SDK work and authenticated
inference remain outside this fixture. The accepted binary is still staged.

Final restored-daemon review `f299b5c3` accepts the source and saved evidence
with no findings. Verification `860a35a5` binds freeze `caf09f97`, summary
`cdc939cc`, 65 seals, five published files, 13 unchanged production contexts
and 42 synthetic outputs. Two R1 passes are reused; one corrected R2 case
passes. R1 remains failed overall. Receipt hashes and freeze/summary bindings
were checked without rerunning tests or querying either service. Real-account
selection and the bounded authenticated stream check remain pending.

The new `examples/browserless-routing.json` configuration passes a separate
process fixture for normal streaming dispatch. With an empty account store,
the request reaches native dispatch and returns the expected `Ineligible`
refusal before a host or worker starts. The fixture supplies synthetic
`x-framerize-context` metadata; a real caller needs valid metadata for its own
account and project. Its first two attempts omitted that header and are
retained as failed fixture evidence. The corrected run exited cleanly, with
a 16,809,984-byte group peak, five tasks and no swap or memory events.
The previously accepted binary was reused. Independent review accepted this
source and saved evidence with no findings: receipt `05fc283a`, freeze
`cb8117b6`, summary `aea1130b`. Resource sampling preceded cleanup; daemon join
and an empty systemd ControlGroup do not establish filesystem cgroup absence.
The configuration is now applied to port 8788 as described above. No
authenticated request is covered by these results.

The earlier nonstream admission repair passed its bounded normal-route check.
An unmetered request was disconnected after its browser fetch started. The
current call finished, cleanup completed, and no additional attempt started.
A second request then returned a complete HTTP 200 response without a service
restart. This verifies the bounded cancellation and recovery case, not every
historical 503 or successful browserless delivery.

The user has reopened the admission issue after streaming interruptions. The
saved 2026-10-23 incident contains a successful `HEAD /anthropic/api/hello`
at 10:53:07 UTC, followed by `Unconfirmed` refusals at 10:53:10. Source tracing
shows that Axum strips the HEAD body outside the ownership layer, dropping
the plain hello response with an armed owner. The new server-only correction
marks these three static hello aliases complete, and marks terminal initial
wave/account refusals complete before residency or external work begins.
It preserves the existing response status and cooldown headers. Unknown,
lazy and paid work retain their existing completion requirements.

The exact correction from `27e0d578` and its three new routed regressions are
integrated as `f27f3165` on `browserless`. Source review is accepted and all
22 selected cases pass: the existing 19-case slice plus the three new cases.
The server library and final executable compile successfully. The executable
is `00508774`; independent saved build review is accepted. Deployment remains
pending. The runtime
owner will preserve the current support configuration and verify normal
streaming, a hello HEAD probe, interruption and a subsequent complete request
on one generation. This does not require another offline trial.

The new streaming acceptance client passed 14 local fixtures. They exercise
fragmented frames, terminal errors, incomplete HTTP framing, cancellation of
the real response socket after headers, and refusal to label a completed
response as cancelled. Its two inference requests stay on the normal route.
The exact source delta is published in
[admission-stream-client-r1.patch](../reference/analysis/admission-stream-client-r1.patch).
The corrected source and concrete deployment bindings are accepted. No live
request or service change followed this fixture run.

Client review found that a raw account-repick reason could reach a saved
failure report. The additive correction retains only the three known
admission-refusal reasons and omits other raw reasons. Its one new privacy
regression passes; the 14 unchanged framing, socket and request cases are
reused. The original rejected source is preserved. The correction is in
[admission-stream-client-privacy-r2.patch](../reference/analysis/admission-stream-client-privacy-r2.patch)
and has narrow review acceptance. Its final deployment binding is also accepted.

The binary-only deployment continuation has 17 passing focused cases across
saved runs. It keeps the installed host, support and launch files, adopts only
the exact abandoned maintenance marker, and checks startup, cooldown
restoration, account isolation, drain and zero swap before releasing that hold.
The startup receipt now summarizes the complete source map by count and hash;
every source is still verified. Two fixture corrections and the contained
package-consumer refusals remain recorded. Those consumer fixtures substitute
saved service policy and installed-file pins; live checks remain required.
Deployment source and exact plan review are accepted. The second coordinated
attempt and its rollback are recorded below; its streaming client never ran.

The client release binding and owner sequence add 13 passing inert cases:
five for the actual release receipt, one for one-time client execution, and
seven for deployment-before-client ordering and sealed preparation. The
existing 15 streaming/privacy cases and two unchanged terminal/cancellation
cases are reused. Final client/owner review is accepted with no findings.
The coordinated live sequence remains pending fresh quiet acknowledgements
and every existing runtime guard. No additional inference probe is planned.

The first coordinated owner attempt was refused by the disk guard before
the deployment controller started. No service transition or inference request
occurred. Its result remains preserved. Removing only old, unused compiled
startup-cache files recovered 335 MB while retaining browser profile data.
A fresh attempt used byte-identical preparation and sequence code in a new
output directory, with accepted binding review. All original source, build,
client and deployment results remain valid and unchanged.

The second attempt installed the candidate, then rolled back after startup
verification failed. Five candidate prewarms succeeded; the sixth failed.
All six rollback prewarms succeeded. The exact outcome remains
`rolled_back_pending_independent_verification`, with the maintenance hold
retained and no streaming acceptance request sent. Saved traffic shows normal
inference during startup, so the next coordinated attempt also requires the
runtime owner to remain passive. The sixth-prewarm cause is not established.

A separate rollback verification and hold-release helper is prepared in
[admission-stream-rollback-release-r1.patch](../reference/analysis/admission-stream-rollback-release-r1.patch).
It starts or stops no service. Six inert barrier cases pass, plus three cases
for consuming the actual saved validation format. The latter correct an
unreleased prelaunch check that expected `passed` instead of `accepted`.
Independent review accepted the exact source and saved evidence. The one
coordinated run then refused release at the zero-swap check, before either
mutation. A subsequent bounded read confirmed the saved rollback generation
with 265,490,432 bytes of current and peak swap and zero memory events.
The maintenance marker remains held; no release or inference acceptance is
claimed. All quiet peers were released after the owner command completed.
The user requested T3 update validation first. No further Framerize deployment
will overlap that work; the original source, build and client remain accepted.

The admission repair on `fix/live-admission-503-20261023` adds completion
evidence for assembled nonstream Responses bodies and stops future attempts
when their HTTP waiter disconnects. An unread completed body previously
marked the operation gate permanently unconfirmed. Unknown cleanup still
retains its fence. Refusal events now distinguish capacity, seal and unknown
completion without logging request data. All 19 selected regressions now pass,
including the three new nonstream cases. The first run passed 18 of 19 cases.
The unknown-cleanup fixture omitted the existing second-account failover reply.
The correction supplies that reply and retains earlier cleanup uncertainty
even if a later account succeeds. The corrected source has independent review
acceptance and is deployed in R12. The local rustfmt component is unavailable.
Review found a retry-admission early return that bypassed the accumulated
cleanup outcome. Its correction and focused case preserve
its status, body and retry headers. The intermediate snapshot remains unrun.
Core, API and store builds are verified reuse. The host, server and final
executable were rebuilt successfully on 2026-10-23.

The runtime owner has integrated the final correction as `2fed7797` and added one
host event after the in-page fetch starts. The event contains only the job
identifier and route. The corrected 19-case run used a peak of 16.4 MB,
with no recorded swap or resource events. The built executable is `e0ff2db9`.
Independent saved-results review accepts the build and its 19 selected cases.
The normal-endpoint release retains the current R9 support
package and worker configuration; the separate R11 device handoff remains
undeployed. Live acceptance passed on 2026-10-23 at 04:14 UTC with exactly
two requests on one service generation. Account isolation was unchanged.
The 103 samples recorded 4.27 to 4.71 GB of shared browser-service memory,
with zero swap or memory events. This establishes no memory saving.

The frozen R12 operational delta is published in
[admission-repair-release-r12.patch](../reference/analysis/admission-repair-release-r12.patch).
Independent review accepted the source, exact plan and two-request client.
The coordinated deployment and result checks passed, and the owned maintenance
hold was released before the live requests. Earlier failed evidence is retained.

The owner's 03:32:48 UTC diagnosis found the previous binary in a new service
generation (main 2270461, started 03:28:49). Health and pool returned 200.
Those observations do not verify real inference or establish the cause of
the user's earlier request. The later R12 live check above verifies the focused
repair. Next: carry the accepted native-device handoff into an additive release
that retains this running repair, then continue the single browserless trial.
Reuse the completed build and device-hook checks.

The original disconnect correction was deployed from `browserless`. R9 completed its
fresh admission, drain, startup and release checks on 2026-10-23. The maintenance
hold is released. A read-only check after the first trial confirmed that health
and the account pool still return HTTP 200 on the same service generation.
Ordinary traffic remains browser-based, with the native path behind the trial
endpoint. The first real native request returned HTTP 503 in 11 milliseconds.
It delivered no answer. Its exact error code was not retained by that checker.

Review of the
first version found that the browser helper could still retry after disconnect,
one timeout was not treated as uncertain cleanup, and refunds could start after
cancellation or hide uncertain completion. The additive correction addresses
those three paths. Independent source review accepts the correction. All 27
focused checks pass: two actual helper-loop cases, five API cases, four host
cases and 16 routed-response cases. The host group reused its compiled binary
after adding a missing existing fixture to the isolated test package; the
original failure remains recorded. Five production builds now pass, reusing the
matching saved core library. The resulting executable is built and verified.
One API build hit the intermediate file-size cap; the original production
compiler setting passed within the unchanged limits. Both records are kept.
Independent review now accepts the built repair and the exact R9 release plan.
The first client check sent zero requests: its ownership check expected a
loopback-only listener, while the service owns a valid IPv4 wildcard listener.
A one-comparison correction passes four focused ownership fixtures; its
regression reproduces the previous refusal. The original 16 cases remain
reused evidence. That correction was accepted before the request was sent.
A later, accepted client change records only known error codes from the exact
application error envelope, preserving response privacy and all existing gates.

The trial selected an account already serving browser requests. Native admission
requires an absent browser. The five unused accounts also lacked a recorded
primary device. Closing a browser previously lost its current device selection
instead of transferring it back to the shared lifecycle.

The new host fix preserves the actual selected device after confirmed browser
closure. Four focused checks pass, using the actual closure method; the new
regression fails against the old support module. Unconfirmed closure still
refuses native work and the existing cooldown history remains intact. Independent
source and saved-result review accepts this fix at commit `49256e89`.

R11 now has independent source and saved-evidence acceptance. It preserves the
rejected R10 package and corrects its stale privileged-helper pin and rollback
decision for two releases using the same executable. Six focused entry and
binding checks pass. Rollback retains the new restoring host and support files;
it restores the original executable and launch configuration.
The accepted device fix reuses the built application and changes its support
module plus the generated host's reference to that support set. The three
package checks, current manifest-consumer check and existing builds are reused.
The exact operational source delta is published in
[device-handoff-release-r11.patch](../reference/analysis/device-handoff-release-r11.patch).
R11 remains undeployed. R14 carries the device handoff onto the verified R12
repair and retains its executable. R13's baseline check correctly refused an
incomplete list of protected configuration files. R14 preserves both the R9
and R12 file sets; its read-only preparation and baseline capture now pass.
The package and manifest checks are reused from their unchanged R13 inputs.
The frozen operational delta is published in
[device-handoff-release-r14.patch](../reference/analysis/device-handoff-release-r14.patch).
Independent review accepted R14, its exact plan and the one-request client.
R14 deployed on 2026-10-23 at 04:37 UTC and released its maintenance hold.
The chosen idle browser was parked and its closure confirmed. One browserless
request then returned HTTP 502 with failure kind `Upstream` after 2.546 seconds.
It produced no accepted answer. The same service generation still returned
200 for health and pool admission afterward. No request retry was made.
The running endpoint discards its detailed upstream and cleanup evidence.
A diagnostic change now retains safe status, attempt and cleanup fields in
the existing event log after completion. All three focused HTTP cases pass,
including field privacy and final cleanup selection. The application links
successfully as `162105d5`, reusing five unchanged library builds and all 32
direct external libraries. The link used a 768 MiB cap and peaked at 397 MB,
with no swap or resource events. An earlier attempt refused memory admission
before compilation; that refusal remains saved. Request and retry behavior
are unchanged. Independent source and saved build reviews are accepted.
R15 retains the R14 host, support package and worker settings.
Its build consumer accepts the diagnostic build and rejects the older summary.
Independent source, plan and client reviews passed. R15 deployed and released
its maintenance hold on 2026-10-23 at 05:10 UTC. One native request returned
HTTP 502 after 3.072 seconds. The new completion event identifies an upstream
403 security-envelope refusal, one authorization and transport attempt, and
confirmed final cleanup. This does not identify the exact rejection cause.
Health and pool admission subsequently returned 200 on the same generation,
with no uncertain residents, swap or resource events. No request was repeated.
The worker now uses the same truthful `Framerize/0.1` application identity as
both native HTTP clients, replacing JSDOM's default browser label. Its page
context is unchanged and all resource fetches remain disabled. Two inert
JSDOM tests passed without executing the SDK or sending a request. The saved
resource collector used an empty cgroup path and its metrics are invalid;
an additive correction preserves that failure. Configured limits and test
exit success are recorded, but no measured test peak is claimed. The test
unit is absent after cleanup. This source change is not deployed and is not
proven to resolve the upstream refusal. Independent source and saved-evidence
review accepted commit `02899fcd` with no findings. R16 packages that change
while retaining the exact R15 server, host and support files. Its read-only
preparation passed with 1,718 inputs and eight preserved configuration files.
Source, plan and client reviews accepted the frozen package. Its operational delta is
[worker-identity-release-r16.patch](../reference/analysis/worker-identity-release-r16.patch).
The 2026-10-23 attempt passed readiness but failed startup. After four browsers
warmed successfully, the memory guard refused two more allocations. Rollback
also recorded one failed warmup, so the controller retained its maintenance
hold and returned failure. No browserless request was sent.

The recovery generation subsequently returned HTTP 200 for health and pool.
Separate inspection verified the original launch configuration, restored
cooldowns, all 11 account identities, six resident slots and no uncertain or
closing residents. Both failed startup records remain unchanged. A narrow
helper to release only the verified pending handoff and owned maintenance hold
received source acceptance. It cannot restart the service or send an inference
request. Its first execution refused on service swap before either mutation.

The first helper review found that original arguments and absent new override
files did not prove the eight preserved installed files were unchanged. The
additive correction reuses the existing protected-file and complete override
inventory check before either bookkeeping mutation. The earlier inspection
remains explicitly partial. The exact helper source is preserved in
[r16-recovery-hold-release.patch](../reference/analysis/r16-recovery-hold-release.patch);
its narrow closure review passed. This patch records source, not a reusable
authorization to release a different generation or maintenance owner.

A separate installed-runtime recovery was reviewed and completed. It preserves
the current R15 payload and existing maintenance ownership, reusing the accepted
drain, capture, settlement and startup paths for one controlled restart.
Release still requires successful warmup, unchanged account and protected-file
bindings, idle residency and zero current/peak swap or resource events.
All eight inert recovery cases pass. The first run's mock-spec error remains
preserved. On 2026-10-23, the coordinated recovery completed with six successful
warmups, all 11 accounts preserved, six idle residents and no uncertain cleanup.
Current and peak swap and memory events were zero; the owned hold was released.
The runtime owner stayed in one passive tool call throughout the transition.
All three peers received an early quiet release. This recovered the installed
R15 version; it did not deploy the R16 worker or make an inference request.
The initial local result check compared whole snapshots instead of generation
identities. Its refusal is preserved; the corrected check verifies matching
main, sidecar, boot, cgroup and invocation identities across the saved result.
The source is recorded in
[r16-runtime-recovery.patch](../reference/analysis/r16-runtime-recovery.patch).

R17 has source, plan and client acceptance against that recovered generation. It retains
all seven R16 worker files and the R15 binary, host and support package.
Preparation and baseline capture verify the exact recovered processes before
accepting the release inputs. The 82 local seals and 148 source declarations
preserve the failed attempts, recovery receipts and eight protected files.
The source seal is `015fca72`; the manifest is `04a400a7`.
Its source delta is published in
[worker-identity-release-r17.patch](../reference/analysis/worker-identity-release-r17.patch).
Its first attempt stopped before controller launch at the unchanged disk
reserve guard: 873,046,016 bytes were available, below the strict 1 GiB floor.
No deployment, rollback or request ran. The refusal remains preserved, and all
three peers received an early release from their quiet holds.
Verified package-index cache and one redundant browser download were removed,
recovering 333,922,304 allocated bytes. Installed packages, the extracted
browser, profiles, runtime state and frozen evidence were preserved.
At 06:52 UTC, 1,169,182,720 bytes were available. This is modest headroom,
not fresh deployment admission. A separately recorded second attempt passed
fresh readiness, drain, startup, preserved-state and owned-hold release checks
using the unchanged accepted plan. All six warmups succeeded, all 11 accounts
were preserved, and the saved startup result has zero swap and memory events.
The owner remained in one passive tool call for the 243-second operation.

R17 is deployed. Its one browserless trial returned HTTP 502 after 3.02 seconds.
The completion event records one authorization and one completed transport:
upstream HTTP 403 with the same security-envelope classification as R15.
Both inner and final cleanup were confirmed. The worker identity change did
not resolve that refusal. No answer or attributed memory saving is established.
Saved result verification is `b37578a5`; the client result is `e811383c`.
All quiet holds are released. Next: trace the remaining token and native
request-context differences from the preserved sources, without repeating
the same request or deployment.

The built candidate carries the worker's actual saved device into
the existing per-request cookie context. Previously the worker seeded that
device, while the subsequent native request used only the saved login cookie.
The candidate preserves the original login and private session, rejects
conflicting or duplicate device cookies, and retains account generation,
freshness, cancellation and cleanup checks. Three new focused cases cover
the cookie boundary and joined-mint handoff. Source review accepted the
correction, and all 33 selected API cases passed in a separate fixture that
preserves their bodies and all production bytes. Its 375 excluded cases remain
explicitly unrun. The fixture, production API and executable builds passed
their receipt checks; unchanged core, host, store, server and worker libraries
were reused after source comparison. Saved summary is `beef996b`; candidate
binary is `9e03bde3`. No swap or resource event occurred during these stages.
This addresses a context gap; upstream acceptance remains unverified.
Independent source and saved-results reviews accepted
this exact build. R18's release-source review is accepted, with
82 local seals, 165 declarations and 11 modules. Its saved-file consumer
accepts the new build and rejects the older completion summary. The host,
support, worker and service limits remain unchanged. Ten earlier config and
override files stay protected. The exact final plan and client reviews are
accepted. R18's first coordinated attempt stopped before controller creation
because free disk was 988,340,224 bytes, below the unchanged 1 GiB reserve.
Readiness passed on the existing R17 generation; no staging, restart or request
ran. The refusal is preserved and all three quiet peers were released.
Removing 1,751 old, single-link Bun registry metadata files recovered
131,002,368 allocated bytes. Installed packages, source, runtime state and
bound evidence remain intact. The saved inspection found no visible use or
installer, with restricted process mappings explicitly unobserved.
Free space afterward was 1,099,165,696 bytes, only modest reserve headroom.
A separate cleanup removed 18,646 single-link files in three old Bun package
caches, recovering another 224,501,760 allocated file bytes. Current references,
home symlinks and visible process use showed no dependency on these copies.
Earlier cleanup receipts referenced only previously removed files. Restricted
process mappings remain an explicit observation limit. Free space afterward
was 1,197,637,632 bytes. Both cleanups are complete and their results retained.
The separate R18 attempt deployed successfully on 2026-10-23 at 08:03 UTC.
The owned hold was released and the selected idle browser was parked with
confirmed closure. One browserless request returned HTTP 503 `Ineligible`
in 8 ms with complete framing. Its completion event confirms cleanup but
contains no attempt details. Saved release and client identities agree.
No browserless delivery or memory saving is established.

The preserved handoff snapshot contains no primary-device selection for the
selected account. Source tracing found that the capture code drops a parked
account's retained selection: it reads the primary only from resident browsers.
Warmup can succeed without filling this missing selection. The exact refusal
predicate was not logged, so the original incident attribution remains limited.

An additive capture correction preserves the lifecycle's recorded selection
for parked accounts while keeping the actual resident browser authoritative.
Null and missing selections remain unknown; cooldowns and request histories
are unchanged. All 17 extractor cases pass, including four new boundaries.
The original extractor reproduces the parked-device loss with otherwise equal
output. The isolated run peaked at 34.1 MB, with two tasks and no swap or
resource events; its unit and cgroup are absent after completion.
Independent review accepts the capture source and saved results. It cannot
reconstruct an already-lost selection. The next live step needs an actual
current selection from the existing account owner. No deployment was repeated.
The exact change is in [parked-device-capture.patch](../reference/analysis/parked-device-capture.patch).

The separate R8 client prepares that selection with one small normal free
request, confirms its completion evidence, then parks through the pool owner
before invoking the unchanged native client once. It stops on changed service
generation, unavailable accounts, ambiguous events or incomplete responses.
All 16 new inert cases pass, with a 28.1 MB peak and no swap or resource events.
The fixture unit and cgroup are gone. Independent review accepts the exact
source and saved results; the existing client results are reused.
The first coordinated attempt stopped before either request: the current
service had swap. A separate resource snapshot at 08:30 UTC recorded 3.74 GB
resident and 587 MB swapped, including the main server and sidecar, with no
memory-limit events. The R18 generation was unchanged. No request or service
action ran, and all quiet peers were released. This attempt is preserved.
The next step is a separately reviewed recovery of the same installed payload,
followed by preparation and the native trial in one continuous quiet window.
That recovery candidate passes all 14 inert cases, with a 17.7 MB peak, one
task and no swap or resource events. It preserves installed file bytes and
identities and uses the accepted parked-device capture correction. Review
rejected one uncovered boundary: launch capture still expected the executable
from before R18. The exact original source is preserved in
[r18-installed-runtime-recovery.patch](../reference/analysis/r18-installed-runtime-recovery.patch).
The additive correction passes the verified installed R18 binary explicitly
to a recovery copy of capture. Inherited deployment and rollback constants
remain unchanged. Four new inert cases cover the actual preparation call,
wrong executable hashes, invalid current pins and the original refusal.
All four now pass, with a direct cgroup peak of 17.3 MB, one task, zero swap
and no resource events. Their unit and cgroup are absent. The original 14
cases remain reused evidence. Independent source review accepted the exact
correction at `754baf2`; its source-only receipt cannot authorize recovery.
The final execution plan separately binds the four-case result `e53852b4`
and has independent acceptance, conditional on all fresh runtime predicates.
No service transition or request has occurred.
Two memory refusals, the coordinated disk refusal and two timed-out cache
audits remain preserved. Subsequent audited batches recovered 81.1 MB and
201.6 MB of old single-link cache files. Installed hardlinks and preserved
inputs were excluded; restricted process mappings remain an audit limit.
The test launch passed the unchanged memory and disk guards. All 17 source
files still reconstruct exactly from the additive patch.
See [r18-recovery-capture-r2.patch](../reference/analysis/r18-recovery-capture-r2.patch).
The follow-up client now binds the future recovered generation while keeping
the original R18 release as provenance. It checks successful controller
completion, unchanged runtime/support and protected files, zero swap/events
and the released hold. The normal preparation and native request must share
that verified generation. All 25 focused client cases pass, including the old
driver's generation-refusal regression. Peak memory was 27.6 MB with no swap
or resource events, and the test unit is absent. Review rejected a comparison
between wrapped protected evidence and the state's bare fingerprint. Its
fixture used the bare value everywhere and missed the actual producer shape.
No recovery or live request ran during this work. The original source is in
[r18-recovered-client.patch](../reference/analysis/r18-recovered-client.patch).
The separate correction binds the three complete wrappers to the state's
fingerprint and release, retaining the live bare-fingerprint comparison.
The fixture now calls the actual producer. The original-client refusal and
changed fingerprint/release cases pass. All 32 focused cases pass, including
five owner-orchestration checks, at a 28.5 MB peak with zero swap or resource
events. The test unit and cgroup are absent. The original 25-case result and
rejection remain preserved. A disk refusal preceded this run; a separately
audited cache cleanup restored the unchanged reserve. No runtime recovery
or request has run. Independent review accepts this exact source and result.
See [r18-recovered-client-r2.patch](../reference/analysis/r18-recovered-client-r2.patch).
The first owner sequence stopped at administrator authentication before its
controller or either request started. Its result is preserved. A bounded
terminal check isolated the launch boundary: a new child session loses the
cached terminal authentication. The separate owner correction keeps the
terminal session while assigning the child its own process group.
All seven focused owner cases pass, including the original terminal-loss
regression and cancellation of the corrected child and its descendant.
Peak memory was 34.4 MB across five tasks, with zero swap or resource events;
the fixture unit and cgroup are absent. Existing client and recovery evidence
is reused unchanged. Independent review accepted the owner correction. Its
single coordinated attempt passed authentication and the quiet/cooldown wait,
then failed to import the recovery identity helper. It acquired no maintenance
hold and started no service transition or request. The outcome is preserved,
the controller is gone, and all quiet peers were released.
See [recovered-owner-terminal.patch](../reference/analysis/recovered-owner-terminal.patch).
The separate recovery R3 changes only that helper's binding import. All 17
existing recovery source files remain exact. Both focused loader cases pass:
all 14 selected helpers load without live actions, and the original helper
reproduces the missing import. Peak memory was 26.8 MB with one task, zero swap
and no resource events; the fixture unit and cgroup are absent. The original
18 recovery/capture cases are reused. Independent review accepts the exact
correction and its separate plan, conditional on every fresh runtime check.
It proves no live recovery or browserless response. See
[recovery-identity-import.patch](../reference/analysis/recovery-identity-import.patch).
The separate client/input/owner rebind changes seven constants across six
files. Static comparison preserves all other bytes, and the published patch
reconstructs those files. It reuses the accepted client and terminal-owner
results without a rerun. This rebind awaits narrow source/binding review
before a new quiet window and one guarded attempt. Both consumed owner
attempts remain preserved. See
[recovered-client-r3-binding.patch](../reference/analysis/recovered-client-r3-binding.patch).
The original preparation client's first test launch refused disk reserve. A separate
audited cleanup removed 135.4 MB of old, unreferenced package-cache copies;
the original refusal remains preserved. No service action was involved.
See [device-preparation-client-r8.patch](../reference/analysis/device-preparation-client-r8.patch).
Missing log events remain a limitation, not proof that no other attempt ran.

The operational delta is published in
[saved-device-cookie-release-r18.patch](../reference/analysis/saved-device-cookie-release-r18.patch).

R17 reused the existing builds and focused tests. The independent test cleanup
remains separate, with 27 batches awaiting later owner validation.

Saved events also show ordinary inference during both startup windows.
Client acknowledgements alone therefore did not establish continuing quiet.
Before another deployment, address that activity and its memory demand;
do not repeat the failed attempt or relax the memory reserve.
The exact operational source delta is saved in
[completion-evidence-release-r15.patch](../reference/analysis/completion-evidence-release-r15.patch).
No browserless live response or total-memory saving is verified yet.

The first version's core and API builds remain saved evidence for that version.
Its original core evidence check omitted an already-pinned source; a separate
reconciliation preserved the rejection and reused the build. Those builds do
not establish that the later correction passes.

The R8 controller completed its service replacement. Its final checker then
rejected a valid six-file runtime package because it still expected five files.
That checker is corrected in a separate copy; the original failure is preserved.
All 42 bounded checker cases pass, including the actual six-file manifest.
Independent source and saved-result review accepted the correction with no
findings. Review SHA-256:
`c93c66a37a61c324e38ba7d629d920adc9b5ecb45c504db9eec4ea4ea80f8c68`.
The exact operational source delta is recorded in
[r8-completion-checker.patch](../reference/analysis/r8-completion-checker.patch).

External recovery subsequently restarted the service and disabled the offline
worker option. Read-only checks after that recovery found the reviewed binary
and sidecar installed and the service healthy, but a different process generation.
The old maintenance marker is absent. Archived handoff bytes match the
original evidence; the corrected historical checker passes all its checks.
Independent review accepted that archive-based result as historical evidence.

That R8 evidence remains historical. The separate R9 deployment described above
reconciled the recovered generation and restored the trial endpoint while
preserving browser traffic and cooldowns. Earlier preparation and component
results below retain their original scope.

| Branch | Purpose | State |
| --- | --- | --- |
| `main` | Normal browser-based path | Verified browser source restored to Git |
| `browser-based` | Maintained browser fallback | Same browser baseline |
| `browserless` | Native request path and SDK worker | Integrated and locally checked; real delivery remains |

## Browser baseline

The source comes from the saved `pool-lifecycle-validation-r3` snapshot.
All 89 source hashes were checked before import. The accepted saved lifecycle
results cover 15 phase sets, 197 Rust cases and 27 separately reused Python
cases. Those results were reused, not rerun for this publication.

The preserved browser baseline binary has SHA-256
`6fb7795117f1ec7c49f06d9453c0466b6468bbc257326715bb3bccf90587ea67`.
The original preparation hash is
`ac8b3c00d8a27e7f81836d436a1a153b157cd3379c538185988cf3566cdf6911`.
The accepted saved-results review hash is
`32abcb23db4f6c1903db45225b6b03b914c62995f357274ee1d5230b484b6fdd`.

The runtime cooldown-restoration overlay is separate from the embedded base
Python host. Preserve that overlay and the original cooldown timestamps in
any subsequent deployment.

The published `scripts/prepare_browser_runtime.py` reproduces the deployed
sidecar and support-manifest hashes exactly from tracked source. This was
checked offline during publication, without importing or running the host.

## Browserless progress and limits

This branch now contains the accepted `offline-client-routing-r8` source.
All 282 original source hashes were verified on import. The new repository
instructions were retained instead of restoring the old `AGENTS.md`.
Source-review hash:
`4df30f4e178e984cb4cb8ed14e26bc9a8c683891c48ff73149fce43a62ab1f5b`.
Saved routing-results review hash:
`0f89cb75beb5dc95e75c8066dbd1036a8acea6dd256d0fe3889437fd5c2b379a`.
These are reused results for the original source, not a fresh Git checkout
build or a deployment. Some isolated worker fixtures still name local paths.

The earlier serving foundation passed 882 component cases. Later focused
routing validation passed 49 API cases and 38 application cases, with 67
application cases filtered out. The routing binary is built and undeployed.
These results do not establish a new full-suite pass for the later source.

The routing worker passed 19 cases. A separate real SDK mint succeeded with
the token discarded; it did not load account state or send an upstream
inference request. Neither result proves end-to-end client delivery or the
total-memory target.

The saved-device worker and shared device/rate admission components passed
their prepared 28-case and 23-case suites on 2026-10-22. Peak test memory was
78,086,144 and 34,054,144 bytes respectively, with zero recorded swap or resource
events and complete teardown. These were inert checks using temporary state.
They did not run a live SDK mint or inference request. The host/application
connection now has separate focused validation, described below.

Saved result hashes: worker
`f5e763424a839450e0719b112c0b12811f4995d0c30ac320985c74e55270ebd2`,
native device/rate
`55f251eea0cd09d24e9bea31295a2b2f52fe8a3e8c0a48dd00c5f5bcf2a94b38`.

The application now keeps the restored host's device grant through native
session, worker and response cleanup. Each mint uses the same device/rate
authority as the browser path. A cancelled waiter cannot release running work.
Unknown cleanup retains the account fence. Successful stream headers remain
available before body cleanup completes.

The first integrated checks missed a Rust/JavaScript device-format mismatch.
That error is fixed. A regression now sends Rust's actual encoded bytes to the
unchanged JavaScript consumer and rejects the former incorrect representation.
The corrected source passed independent narrow review and six fresh build/test
phases on 2026-10-22: 30 worker cases and 12 focused application cases passed.
The application run filtered 97 unrelated cases. Five unchanged libraries and
the 23 device/rate cases reuse their original results and exact dependencies.
The final executable built successfully. One disk-reserve refusal was resolved
by removing unused package cache; no memory limit was raised.

Independent review found that the two application builds had not recorded the
new worker library as a direct input. The commands used the right library,
but their dependency records were incomplete. The corrected records now pin
every directly used library. Both application outputs were rebuilt and match
the earlier binaries exactly, so the original test results remain reusable.
No component suite was repeated for this correction.

Corrected integrated result SHA-256:
`5202e63374500c50b96b1ba17bf977826ca218bd7c8ef72f67edf866414677fb`.
Source-review SHA-256:
`23b897870d6b39837d7b2d7d025d2f6ae2fe69936f6a59b62907bc82ddca8232`.

Independent review now accepts the corrected build evidence and the R6
release source and evidence bindings. The acceptance hashes are
`8b7ea21ccd4cb84b3751a167a7beaa6430c57bcfddaddf47ac0411e044dcb7a4`
and `6b25c5ebb383673825b96477ddbd9fd2deacb2ac9853d7c74121cd2cc5638640`.
The concrete R6 plan and its single-request checker are independently accepted
within their source and evidence scopes. The checker uses that plan and the
corrected executable. Only its three release constants changed. The accepted
framing logic and original 16-case evidence are reused without another test run.

This milestone is built and locally checked, not deployed. No real
browserless response or whole-service memory saving is established. A fresh
readiness check refused before staging because one system DNS file changed
from the saved runtime inventory. The other 2,718 inventory entries match.
The additive R7 release records the current DNS file and keeps every other
inventory row, application binary and execution control unchanged. Its narrow
source review is accepted:
`c2d2c7b3404f96640607b9d8e1efa5812775354dd2570c81e734346fb5179c9c`.
The original refusal and R6 plan remain preserved.

The concrete R7 plan and its client-check binding are independently accepted.
Plan-review SHA-256:
`d841c43c04745e34533ba891f6b805fd239747d16ca5181e4ce04ce2c769705d`.
Client-review SHA-256:
`837fb3b3b0e82fa65e4d31c12f46071cc1258b69d03e7c4afc49b456dea21367`.

At 21:18 UTC, fresh read-only admission passed the input checks but refused
the startup window because four account cooldowns would expire within it.
The service was not changed. The accepted window remains 440 seconds plus
the actual 2.5-second settlement; account cooldowns remain untouched.
Administrator authentication is also required before a service transition.

A second sample at 21:26 UTC found new cooldown deadlines. A focused read-only
comparison confirmed that the six selected startup accounts would change
within the required window. This is a real admission constraint. The original
cooldown timestamps and startup checks remain unchanged.

The one-request input is prepared privately from the host's saved editor
target, matched to the current account and project. It starts a new test
conversation and limits the response to 16 tokens. No request has been sent.

At 22:36 UTC, the authorized R7 terminal launch authenticated successfully.
It then refused before staging because the service did not drain within
55 seconds. Both saved samples during that attempt had four active requests.
The controller released its maintenance marker and finished cleanup. The
original browser service remained active with the same process identity.
No installation, restart or browserless request occurred.

The coding sessions themselves use this Framerize service. Their request
traffic must be accounted for when arranging a quiet deployment window.
The consumed R7 launch is preserved and must not be rerun in place. The next
step is to coordinate that window and prepare a separate execution binding
without relaxing drain, cooldown, capture or startup checks.

The separate R8 source package is frozen, with the same application, worker
and runtime inventory. Its fresh read-only preparation checked 1,709 runtime
file pins. Only provenance, review paths and generated launch paths change.
It reused the original transition and checker results. This preparation did
not establish admission; the later execution and recovery are recorded above.

The final R8 plan and client R5 binding are independently accepted. Their
review hashes are `f8303fe1b25f15842c1d533d119586f2f7f14a5c5b71019e270e46d331cc37f6`
and `f537162f79c757bbee99158754b63d915723df4d7cb5b1778bd234b6df71f53e`.
The four client source entries match their seal. The collected R8 readiness
sample refused before any service change because cooldown expiries fell
inside the startup interval. At 22:49 UTC, the last relevant expiry was
about eight minutes away if no new cooldown appeared. A later coordinated
window admitted the R8 launch. That launcher is consumed and must not be
replayed. The 16-token request remains prepared privately and unsent.

The next live milestone remains a coordinated trial release, followed by
one real request with fresh account checks. This dependency belongs to the
live step; it does not stop the wider engineering goal.
Keep normal browser routes, restored cooldowns and one account authority.
Preserve the original application build and valid component results while
reconciling the changed runtime. Do not repeat a suite merely to publish work.

The continuation instructions now explicitly prohibit voluntarily stopping
the overall goal because a selected step must wait. Before concluding that
work is exhausted, assess independent implementation and measurement work
across streaming, tools, cancellation, cleanup, latency, whole-service memory
and account isolation. Carry out the smallest useful available action and
record its result. Do not count repeated readiness checks as progress.

This documentation correction does not change the recorded deployment or
test results and establishes no new live delivery or memory saving.

The first-request observer is now implemented separately from the accepted
checker. It samples memory, swap and task counts during that same request,
keeps final measurements on failure, and records sampling gaps. Four focused
local checks passed using temporary counters, with no network or child-process
attempts. The original checker and release remain unchanged. This prepares
measurement; no live browserless response or service-memory saving is claimed.
See [browserless-measurement.md](browserless-measurement.md).

The optional observer's timeout-handling bug is corrected locally. All five
focused cases pass, and the new alarm regression fails at both affected sites
against the original code. No network request was made. Independent review
remains pending; this observer will not gate or wrap the first live request.
