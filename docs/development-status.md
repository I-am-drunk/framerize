# Development status

Updated 2026-10-23. The normal service remains browser-based. Publishing these
branches does not change the running service.

The admission repair is now deployed and passed its real normal-route check.
An unmetered request was disconnected after its browser fetch started. The
current call finished, cleanup completed, and no additional attempt started.
A second request then returned a complete HTTP 200 response without a service
restart. This verifies the bounded cancellation and recovery case, not every
historical 503 or successful browserless delivery.

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
and still awaits review. No service transition or request has occurred.
Two memory refusals, the coordinated disk refusal and two timed-out cache
audits remain preserved. Subsequent audited batches recovered 81.1 MB and
201.6 MB of old single-link cache files. Installed hardlinks and preserved
inputs were excluded; restricted process mappings remain an audit limit.
The test launch passed the unchanged memory and disk guards. All 17 source
files still reconstruct exactly from the additive patch.
See [r18-recovery-capture-r2.patch](../reference/analysis/r18-recovery-capture-r2.patch).
Its first test launch refused disk reserve before starting a test. A separate
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
