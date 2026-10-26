# Development status

The separate S1 runtime-ownership candidate at `71581d5` in
[PR #7](https://github.com/I-am-drunk/framerize/pull/7) now compiles and passes
all 351 selected isolated checks: API 13, core 8, native 66, server 252 and
browser application 12. All 765 source hashes match; compilation and both
test units are stopped with no remaining processes. The initial nine fixture
assignment errors were corrected before these runs; production ownership
was unchanged by that correction. This qualifies the separate S1 branch,
not its integration here or live delivery. Frozen `3ead4b8` packages remain
unchanged. S2/S3 are not released. Next: qualify the queued account-retention
correction, then compose S1 with the newer feature changes.

Extra-workspace provisioning now uses the existing UTF-8-safe preview in its
successful-create log. Short or non-ASCII metadata IDs can no longer panic
before the returned project is retained. All four isolated provisioning
checks pass after the parent reproduced two crashes with two passing controls.
All 761 source and three candidate hashes match; both test units are stopped.
Discovery, newest-project adoption, client-call order and full identities are
preserved. This integrates A's `d9eb887` on the experimental feature branch,
outside the frozen `3ead4b8` packages. Live provisioning was not exercised.
Next: resume qualification of the separately corrected S1 runtime fixtures.

The rearchitecture is on `refactor/separate-apps-and-ci-20261025` in draft
[PR #2](https://github.com/I-am-drunk/framerize/pull/2). It is not deployed.

Dedupe reports now end device and team previews at a valid UTF-8 boundary
within the existing eight-byte limit. Empty and short saved IDs no longer
panic during report construction. All five isolated handler checks pass
after the parent reproduced three panics with two passing controls.
All 760 source and three candidate hashes match; both test units are stopped.
Full identity still determines clone/dead classification. Preview checks
confirm unchanged records and no browser, client, session or state writes;
deletion itself is unchanged. This integrates A's `8f79fa4` contextual delta
after the heartbeat and registration corrections, outside frozen `3ead4b8`
packages. The separate provisioning-log follow-up remains source-only.

Responses heartbeats now use typed `response.in_progress` events in both
item-driven serving paths, with the same response ID and sequence as output
and completion. An isolated codex-cli 0.163.0 comparison timed out despite
comment pings but received its delayed answer with typed progress events.
All 24 focused core/server/native checks pass after the unchanged production
baseline reproduced three heartbeat failures with two passing controls.
All 760 source and seven candidate hashes match; both test units are stopped.
Partial text, tool IDs, incomplete responses and joined cleanup are preserved.
This qualified source is outside the frozen `3ead4b8` packages. It does not
establish automatic compaction recovery or live acceptance. Next: integrate
the independently delivered dedupe-preview correction.

Concurrent login and import now serialize duplicate-email validation through
the initial account save. A losing registration cannot save a replacement
session or claim a grant; the shared guard releases before credit admission
and on early failure or cancellation. All twelve isolated checks pass after
the parent reproduced four login/import races with one admission control.
All 760 source and four candidate hashes match; both test units are stopped.
This source fix is outside the frozen `3ead4b8` packages. It protects one
shared in-process policy, without adding database uniqueness or rolling back
earlier session/grant effects after a later failure. Next: obtain fresh live
handover readiness while preserving the qualified packages and service repairs.

Shared Python driver startup now clears the cached start task only while
the finishing waiter still owns that exact task. An obsolete failed waiter
can no longer discard a pending retry, create a duplicate driver or hide
that retry from shutdown. All six isolated checks pass after the parent
reproduced both failures with four controls. All 759 tracked and five
candidate hashes match; both test units are stopped. This is qualified
source on `feat/native-browser-handoff-20261026`, outside the frozen `3ead4b8`
packages. It preserves cancellation shielding and adds no startup deadline.
Next: prepare fresh acceptance from the qualified package and recovery inputs.

Matched browser and native packages at `3ead4b8` now include the qualified
startup and cleanup changes below. Both optimized applications build; nine
browser and eight native package checks pass, together with five packaged
history/readiness checks and three diagnostic/cleanup/privacy checks. All
758 source hashes, twelve artifact/helper hashes and forty supplied-runtime
files match. The seven earlier `f33362b` artifact pins remain unchanged.
All four isolated build/package/test units are stopped with no remaining
processes. These checks use synthetic profiles and make no upstream request;
they do not establish real-account delivery or authorize a service switch.
The packages are undeployed. The subsequent driver-start source qualification
is recorded above; it does not change these archives.

Owned CDP startup now selects the exact Framer origin, bounds WebSocket
connection and upgrade to five seconds, and retains its process group until
exit is observed. Pending command IDs survive cancellation; later work waits
for their replies, and Attach shutdown retains uncertainty when completion
cannot be observed. Startup and mint errors preserve that cleanup signal.
All 49 selected startup/cleanup checks pass: 48 in the combined run and one
after correcting the fixture's retained peer socket. Production and every
assertion were unchanged by that correction; 377 other Rust input hashes match.
The target/deadline baseline reproduced five failures with four controls.
All 749 final tracked and 20 candidate hashes match; both test units stopped.
These source changes are outside the frozen `f33362b` packages. Escaped process
descendants and real-browser behavior remain unqualified. Next: prepare matched
packages without replacing the running service or reusing the old live plan.

Device context creation and preparation now retain the device lock and memory
reservation until outstanding browser calls and late-context cleanup settle.
A failed cancelled close preserves cancellation and marks cleanup unconfirmed,
so the cancelled mint cannot start ambient fallback. All seventeen isolated
creation/preparation/session checks pass. The two predecessor baselines exposed
eight assertion failures and three cleanup-entry timeouts, with six controls.
All 746 tracked and 24 candidate hashes match; every test unit is stopped.
The earlier 23 page-lifetime checks remain valid and were not repeated.
This qualified Python source is outside the frozen `f33362b` packages and adds
no operation deadline or live-browser acceptance. The subsequent CDP startup
qualification is recorded above.

CDP attachment now selects the exact HTTPS Framer app origin before using a
page. Discovery GET and new-tab PUT share an owned async HTTP connection
with a five-second total deadline, 16 KiB header limit and 8 MiB body limit.
Complete fixed-length or EOF-delimited replies are required; failed status,
truncation and unsupported transfer encoding are rejected. Cancellation and
timeout close the socket. All 15 isolated origin/GET/PUT checks pass after
13 reproduced baseline failures and two controls. All 741 tracked and six
candidate hashes match, and both test units are stopped. This qualified
source is outside the frozen `f33362b` packages. Real-browser interoperability
and live acceptance remain untested; chunked HTTP is explicitly unsupported.
Next: integrate the remaining owned-startup and cleanup inputs before making
a new package decision. Live draining still needs effective ingress control
or a freshly coordinated quiet interval.

CDP commands now use one deadline for both writing the command and waiting
for its reply. A stalled socket write can no longer bypass the caller's
timeout, including during browser shutdown. The blocked-write regression
and both reply/error controls pass after the parent reproduced the write
hang. All 740 tracked and two candidate hashes match; both isolated units
are stopped. This source-only change preserves every caller duration and
does not cancel browser JavaScript or establish process exit. It is outside
the frozen `f33362b` packages. The origin/HTTP result is recorded above.

Python browser-page ownership now survives repeated caller cancellation.
Cancelled pool allocation settles and reclaims its page before unlocking;
borrowed pages wait for their own dispatched calls before returning to the
pool. A mint page created after cancellation is closed without navigation
or SDK warming, and a failed close remains an error. All 23 isolated checks
pass across the three changes. Their separate pre-fix baselines reproduced
14 failing methods and nine passing controls; four failures surfaced as
timeouts or cancellation errors rather than assertions. All 734 tracked,
seven candidate and two existing fixture hashes match. Every test unit is
stopped. This is qualified source on `feat/native-browser-handoff-20261026`;
the frozen `f33362b` packages remain unchanged. These helpers add no operation
deadline and do not establish remote JavaScript termination or live browser
cleanup. Next: qualify the remaining CDP connection/startup deltas.

CDP device creation now writes and syncs a separate candidate, then publishes
the complete record without replacing a concurrent winner. Cancelled creation
retains candidate cleanup. Profile and device preparation also retain the
account route until filesystem work completes, preventing same-account work
or shutdown from overtaking it. All eleven isolated checks pass: seven new
publication/preparation cases and four existing identity controls. All 730
tracked and five candidate hashes match; the test unit is fully stopped.
The new private helpers have no raw-parent behavioral baseline. This is
qualified source on `feat/native-browser-handoff-20261026`, outside the frozen
`f33362b` packages. Slow filesystem work can still delay cleanup; power-loss
durability and live browser acceptance were not tested. The subsequent
Python page-lifetime qualification is recorded above.

Owned browser startup now rejects failed stale-port removal before launching
processes, while missing/removable entries and Attach behavior retain their
existing policy. Blocking removal keeps the account route through cancellation,
so same-account reentry and owned shutdown cannot overtake it. All six isolated
guard/ownership checks pass, with 727 tracked and four candidate hashes matching.
The prior guard reproduced one process-dispatch failure with two controls.
The new helper's three cases require its API and have no raw-parent baseline.
This is source-only work on `feat/native-browser-handoff-20261026`; frozen
`f33362b` packages are unchanged. A slow filesystem can still delay cleanup;
no live endpoint, global profile-lock or browser-acceptance claim is made.
The subsequent device-preparation result is recorded above.

Browser version probing now returns the existing fallback user agent after
five seconds instead of waiting indefinitely for output. The timed-out child
remains owned until kill/wait cleanup completes, and probe cleanup cannot clear
an actual browser's uncertainty. Both new timeout regressions and four existing
version/fallback/cleanup controls pass in the isolated runner; the parent
reproduced both hangs. All 726 tracked and two candidate source hashes match.
This source milestone on `feat/native-browser-handoff-20261026` is outside the
frozen `f33362b` packages. It adds no synchronous spawn deadline or live-browser
acceptance. Subsequent port-preparation qualification is recorded above.

Duplicate login and import now validate email conflicts before saving a
replacement session or claiming a workspace grant. Both rejected operations
preserve saved sessions and account records; same-account reauthentication
keeps its existing grant and admission policy. Four new regressions and six
existing import/admission controls pass in the isolated runner. The parent
reproduced two session-replacement failures with two passing controls. All
725 tracked and two candidate source hashes match. This source milestone on
`feat/native-browser-handoff-20261026` is outside the frozen `f33362b` packages.
Concurrent registration and rollback after later failures remain separate.
Next: qualify the pending host deltas while preparing a fresh admission window.

The supervised transition now has an explicit mode for existing schema-2
history. It refuses missing/unknown history and requires the account snapshot
to remain unchanged through native startup. An early abort restores only
owned admission changes under the original invocation; recovery after a stop
still requires a history-ready browser return. Legacy migration is preserved.
All 79 inert checks pass: 12 new and 67 existing controls. The original code
reproduced six failures with four controls. All 724 tracked and three candidate
inputs match. The subsequent existing-history attempt reached its drain
deadline after 181.917 seconds and aborted before stopping the original
service or starting native delivery. Original process, pool and configuration
were preserved. No native canary ran. The packages remain pinned to `f33362b`;
another attempt requires a fresh quiet interval or an effective ingress fence.

Matched browser/native packages and supplied runtime are qualified at `f33362b`.
All 724 source pins and seven package/helper bindings match. Nine browser and
eight native package controls pass, along with five history/failed-host checks
and three diagnostic checks. Both native shutdown modes retain correlated
request/cleanup records without private request fields. These used synthetic
state with no browser, SDK or upstream work. The previous packages are retained.
The repaired live service is unchanged. Its existing schema-2 history requires
a new acceptance controller path; the old first-migration procedure cannot be
reused. Next: implement and qualify that path before any live transition.

Login completion in both browser adapters now reads session cookies applicable
to the API origin. Website-only cookies can no longer replace the API session;
host-only API cookies and shared-domain sessions remain supported. Two Rust and
four Python checks pass after three reproduced behavioral failures and three
passing controls. The initial Python fixture failed before reaching assertions;
its page is now hashable, with the original assertions unchanged. All 722 tracked
inputs and four candidate files match the final run; the qualified Rust inputs
remain unchanged. Journal and native diagnostic changes are preserved.
Next: build matched packages including these corrections and prepare an
existing-history acceptance path from the repaired service. No deployment ran.

Native startup now initializes structured diagnostics after validating selected
accounts, preserving the actual profile path, including non-UTF-8 paths. An
unavailable log emits one fixed stderr warning, never panics or replaces the
blocking file, and can recover on a later event after the path is repaired.
Four new regressions and 23 existing startup/disconnect/diagnostic controls
pass. The old code reproduced three failures; a separate control exposed an
older worker fixture's process-global logger assumption, now isolated without
changing its assertions. All 721 tracked inputs and five candidate files match
the final control run. The four regression inputs are unchanged. This is source
on `feat/native-browser-handoff-20261026`, outside the frozen `8fe2259` packages.
The saved R6 response identifies Upstream but lacks its HTTP status and request
phases; this correction cannot recover missing historical events. Live delivery
remains unverified. Next: integrate the queued login-cookie scope correction
and prepare fresh acceptance using matched packages and the repaired state.

Chrome CDP registration now rejects unsupported embedded proxy credentials
before changing account routes. All five selected inert checks pass; the old
code reproduced two failures with one new control passing. All 720 tracked
inputs and three candidate files match. Ordinary proxy settings and Attach
ownership controls pass. This source-only fix does not implement CDP proxy
authentication or establish egress; frozen recovery packages are unchanged.
Next: trace the saved native 502 before preparing a fresh live-delivery plan.

CDP normal and long evaluations now reject reported JavaScript exceptions
with a fixed error that omits exception payloads. All three local WebSocket
checks pass; the previous code reproduced two false-success failures while
the normal-result control passed. All 719 tracked inputs and two candidate
files match. These fixtures execute no JavaScript or real browser. This
source-only correction is outside the frozen `8fe2259` recovery packages.

Browser session-cookie propagation now reports failed writes, waits for
in-flight device initialization, and preserves update order through caller
cancellation. All ten selected inert checks pass; the previous behavior
reproduced four failures with three controls passing. All 717 tracked inputs
and four candidate files match the qualification. Failed updates may apply a
prefix and require retry; real browser descendant cleanup is not established
by these fixtures. Journal changes are preserved. This source-only correction
is not in the recovery packages built from `8fe2259` and is not deployed.

The rate-history recovery correction is integrated from `8d45b5e`. Browser
and native migration now carry the same configured reserve policy, preserve
existing debt and identities, and recover supported incomplete history only
under verified ownership. Recovery requires account-history readiness from
the expected serving process. The previous R6 return checked HTTP liveness
only and did not establish usable history; its native canary returned 502.
All 183 isolated checks passed in the correction's qualification, and its
32 source/executable pins still match. The 28 integrated source pins match
unchanged. Both optimized applications and matching packages were built from
`8fe2259`; all 717 tracked build inputs remained unchanged. Nine browser and
eight native package checks pass. Packaged readiness accepts valid empty
history and rejects an unknown journal schema while preserving its bytes;
the failed host reports unsuccessful shutdown and leaves no owned processes.
The repaired live service is preserved. These packages are not deployed, and
successful native delivery and full live acceptance remain unverified.

Responses check the existing 300-second active-time budget before explicit
rate, halved-transport and metered retries. All 18 selected checks pass,
including eight new cases; the previous behavior reproduced three extra
dispatch failures with five controls passing. All 707 tracked inputs and
both candidate files match. Exact-boundary and successful in-flight results,
original nonretryable errors and local cooldown subtraction are preserved.
This source change is not deployed. The internal cooldown retry guard is a
separate pending input, and the guarded live history handoff remains open.

Responses emit response.incomplete when their assembled output is incomplete,
preserving content, IDs, usage and event sequence. Direct inference honors
indexed output_text.done, including empty text, and prevents later deltas
from extending a finalized part. All 47 selected checks pass: 36 API, ten
core and one native cleanup control. Thirteen new cases and one strengthened
case ran; the old behavior reproduced twelve failures with two controls
passing. All 706 tracked inputs and six candidate files match. These source
changes are not deployed; the guarded live history handoff remains pending.

Direct inference now orders indexed legacy text by output/content position
and captures it at the last completion. Explicit final output still wins;
ambiguous mixed text or invalid coordinates cannot become a legacy answer.
All 24 selected cases pass, including six new cases. The previous behavior
reproduced five failures with one control passing; all 706 tracked inputs
and both candidate files match. This source change is not deployed.

Responses preserve opaque output items and their positions between messages
and tools. Direct, chained and paid responses reject non-null terminal errors
before publication, while absent/null errors remain compatible. The direct
decoder also treats a bare SSE data field as an empty data field.
All 97 selected checks pass: 52 API, 21 core, 18 server and six native.
All 21 new cases ran; the old behavior reproduced 12 failures with nine
controls passing. All 705 tracked inputs and ten candidate files match.
This follow-up is not deployed. Live delivery, memory/latency measurement and
concurrent account/TLS acceptance remain pending the guarded history handoff.

Responses streams now use one composer for opening, content and terminal
events in both server adapters. Event numbers remain continuous; tool IDs
match their argument/input events and final output. Opening and final response
IDs agree. Failure emits no synthetic completion, including a joined task
failure; native cleanup refusal remains authoritative after partial output.
All 142 selected checks pass: 46 core, 12 API, 78 server and six native.
All 14 new cases ran, after 13 failures were reproduced on the old behavior.
The 702 tracked inputs and ten candidate files match the final run. This
source milestone is not deployed. At 04:35 UTC the original live service and
saved pool remained intact with two active users; live acceptance is pending.

Responses validate terminal content, roles, usage and executable tool fields
before publication. Named failures and malformed terminals cannot hide behind
a later completion. CR framing, valid partial results and opaque fields remain
supported. Direct legacy text ends at completion, and accepted tool call IDs
must be unique. All 217 selected checks pass: 212 API and five native consumers.
All 109 new cases ran; the old behavior produced 71 failures with 38 controls
passing. All 702 tracked inputs and five candidate files match the final runs.
This source change is not deployed. At 04:23 UTC the original live service and
pool were intact with three active users; the genuine handoff remains pending.

Messages streams preserve cumulative usage and metadata, replace opaque
signatures, reject malformed late metadata and named errors, and accept
LF, CRLF and lone-CR framing. Final SSE includes input, cache and tool usage.
All 134 selected checks pass: 90 API, 39 core and five native consumers.
The prepared old behavior reproduced 24 failures with six controls passing.
All 689 tracked inputs and 18 candidate source files match the final runs.
This source milestone is not deployed; live packages remain at `72886b2`.
Live delivery, request-time memory and account TLS isolation remain unverified.

Paid Responses aggregation now rejects missing, malformed and failed terminal
streams, including named errors and CR-only framing. Invalid successful
upstream responses become fixed 502 errors; valid incomplete results and
existing non-success status survive. All 25 decoder/caller/cleanup checks pass.
Two earlier comparisons reproduced eleven legacy and seven framing failures,
with seven controls passing. All 687 tracked inputs and three candidate paths
match. This source change is not deployed and adds no live acceptance evidence.

Responses and Messages now apply refusal policy to the second attempt before
considering paid fallback, with timing measured for that attempt. Completed
Messages streams use the buffered block validator before publication.
The prepared old behavior reproduced 13 failures with ten controls passing.
All 101 selected API checks pass, including four cooldown/cancellation controls;
684 tracked inputs and ten candidate source files match. No live request or
deployment occurred. The live pool still had two active users at 04:05 UTC,
so the existing handoff remained pending without another drain attempt.

Messages account for token usage without narrowing or overflowing counters.
Malformed request bodies return a client error before account work. Free
Responses reject unsupported previous_response_id context rather than silently
dropping it; complete input history and paid passthrough remain supported.
All 59 selected checks pass (33 API, 26 server), after the parent reproduced
16 failures with 11 controls passing. Seven candidate files and 680 tracked
inputs match the final run. These source changes are not deployed and do not
establish live delivery or alter the pinned native/browser packages.

Chat structured-output schemas now retain their constraints and optional
settings in every Responses continuation request. Malformed schema wrappers
return an error instead of losing the requested output format. All thirteen
selected checks pass (four core, nine API), after the parent reproduced three
failures with one control passing. Three source files and 680 tracked inputs
match. Provider schema support and live delivery remain untested here.

Messages preserve citations and rich-block metadata through merging and SSE.
The API Responses chain rejects token-count overflow before publishing the
offending chunk. The parent reproduced all twelve expected failures, with
four controls passing. All 110 selected checks pass: 71 API, 34 core and five
native continuation/tool/stream-cleanup controls. Eleven candidate source
files and 676 tracked inputs match both final runs.
This source work does not change the pinned live packages or establish live
delivery, upstream streaming, memory savings or distinct per-account TLS.

Messages reject zero configuration limits before dispatch, and Responses JSON
and final SSE consistently report aggregate total_tokens while retaining usage
metadata. All 72 selected checks pass: 41 API fallback cases and 31 core
conversion cases. The parent reproduced nine failures with five controls
passing. Six candidate files and 674 tracked inputs match. The qualified
Responses zero-chunk guard and session-token constructors remain intact.
These source changes are not deployed and leave the pinned live candidate alone.

Owned CDP startup and version-probe children now remain owned through
cancellation until their exit is joined. Abandoned shutdown waiters retain
the cleanup result, and a probe exit cannot clear browser uncertainty.
All 17 selected startup/probe/shutdown checks pass, including disposable
processes whose PIDs are absent after failed or cancelled startup. The five
source files and 670 tracked qualification inputs match. These tests prove
the owned child paths, not real browser descendant cleanup or live delivery.
The source changes are not deployed; pinned live packages remain unchanged.

Saved CDP device records now survive read, decoding and field errors instead
of being replaced with a fresh identity. Chat keeps content-filter completion
reasons and supplied function strictness through both continuation requests.
All 37 selected checks pass: 25 core, eight API and four device-record cases.
Parent comparisons reproduced six failures with three controls passing; all
six final source files and 668 tracked qualification inputs matched. This
source batch preserves damaged records for recovery and does not repair them.
It is not deployed and adds no condition to the pinned live candidate.

Responses now reject a zero chunk limit before sending or publishing output.
The parent reproduced a fabricated empty completion; the new six-configuration
regression and three existing completion/fallback controls now pass. This is
a separate four-case source qualification, without a live request or change
to the pinned packages.

Chat responses now retain refusal text through JSON, streaming and subsequent
request history. Owned CDP shutdown reports unconfirmed starts while still
closing known instances. Free completion rejects selected token limits above
u32 before dispatch, preserving precedence, defaults and valid boundaries.
All 44 focused checks pass: 22 core conversions, six API conversions, seven
host shutdown/proxy cases and nine request-budget cases. The eight integrated
source files and 664 recorded tracked inputs matched after qualification.
This source milestone is not deployed. The latest live attempt reached its
drain deadline and restored the original service/pool without a canary;
live acceptance and distinct per-account TLS identities remain unverified.

The next source batch is qualified on the experimental handoff branch.
Responses refuse an initial budget below 16 and stop before rounding a
smaller remainder upward. Buffered Messages reject invalid completions and
keep all token-limited tools behind the existing repair policy. CDP auth
diagnostics preserve UTF-8 boundaries and omit malformed cookie contents.
All 43 affected API checks and four host checks pass. Two budget fixture
setups were corrected and rerun individually; the other 41 API passes remain
valid. Parent comparisons reproduced six API and two host failures with
five controls passing. The live packages remain pinned to `72886b2`.

The native/browser transition recovery helper passes 17 inert regression
tests. They cover pre/post-migration return, retained history, shutdown joins,
concurrent callers, interrupted recovery and unrelated service/pool changes.
The final run took 0.462 seconds under the existing isolated runner.
These checks stub service/network operations and do not prove live recovery.
The pinned packages remain at `72886b2`. No live migration or request occurred
during this helper work; the controller qualification follows below.

The first live supervisor exited during preflight, before changing admission,
stopping the service, migrating history or sending a request. Read-only probes
reproduced a permission error resolving the service executable with
`PrivateTmp=yes`; the identical preflight passed with `PrivateTmp=no`.
The corrected supervisor retains its ownership checks and resource bounds.
Live delivery, memory savings and per-account TLS separation remain unproven.

The corrected supervisor reached its three-minute drain limit without a
service switch or upstream request. Recovery confirmed the original service
and saved pool settings. The revised controller now fences idle accounts
progressively, preserving active requests and recording every attempted pool
change for interrupted recovery. All 47 local transition/recovery checks pass,
including 12 new progressive-drain cases. The deployed pool API's active-work
guard is retained.

The next live attempt completed progressive drain and reached an empty,
fenced pool. Loading the native runtime limit before stopping the original
service caused systemd to expire that long-running process. Recovery restored
the original browser and saved pool; no history migration or canary ran.
The controller now stops and confirms an empty service cgroup before loading
new overrides. Explicit stop phases support recovery before, during and after
shutdown, including partial override installation and interrupted reload.
All 60 focused transition/recovery tests pass in 1.189 seconds; the previous
implementation reproduced ordering and interrupted-stop failures. No live
request or memory measurement occurred in these tests. The next attempt must
use a fresh plan bound to the restored service's current invocation.

The following attempt drained successfully but stopped before override
installation. Systemd reported the original stopped; recovery restored it
shortly afterward. The saved generic error does not identify the failed
shutdown condition. Confirmation now waits up to ten seconds for both service
exit and an empty cgroup, preserving both requirements. Distinct failure types
identify an unsettled service or remaining descendants without private data.
All 64 helper checks pass in 1.256 seconds, including delayed exit, deadline
and unexpected-restart cases. No history migration or canary occurred in that
attempt. The next live attempt remains independent of queued source fixes.

The one-shot transition controller now passes 18 inert tests. It refuses to
stop the original service before an empty, fenced pool; verifies the genuine
expiry record; and durably claims one canary attempt before sending it.
Two synthetic systemd checks confirmed the configured recovery command runs
after both an exit error and SIGKILL. The SIGKILL receipt was normalized from
systemd's symbolic signal name without rerunning the process. These fixtures
used temporary state and did not operate the application service. The next
step is the bounded live drain attempt using the unchanged pinned packages.

Release binaries and both archives are now prepared at `72886b2`. The browser
return package passes nine local startup/lifecycle checks; the native package
passes eight with its packaged worker. Offline bundle assembly and immutable
staging also pass. This supplies a history-schema-compatible return artifact.
No live migration or inference request ran. The next runtime step remains
genuine account-history transfer before the controlled native/browser canary.

The latest combined qualification on `feat/native-browser-handoff-20261026`
passes 154 API tests, 17 core conversion checks and 13 CDP bridge checks.
Response content retains refusal/text order, annotations, logprobs and metadata.
Cached bearers belong to the current session as well as the account ID.
An attached browser bridge retains one account owner through cancellation,
failure and retry. Independent browser engines retain separate admission.
Focused comparisons reproduced all seven content-loss cases, five stale-session
cases and three Attach ownership cases before their corrections. These are
local results; cross-bridge/live isolation and distinct TLS remain unqualified.

The direct inference sender now requires `response.completed` and rejects
explicit failure events. The Responses fallback accepts complete SSE frames
with optional spaces and multiline data while preserving tool output and usage.
Both fixes are integrated on `feat/native-browser-handoff-20261026`. All twelve
focused checks pass. Each old parser reproduced three failures with three
passing controls. These are local parser results; live acceptance is pending.

Direct inference also uses supplied terminal text as authoritative, avoiding
lost or duplicated deltas, and rejects missing response objects and invalid
completion statuses. Older replies without output/status retain compatibility.
The a471fa5/e921c90 stack is integrated with the earlier 401 cache correction.
All 147 integrated API tests pass, including all 16 affected inference cases;
the previous parser reproduced nine failures with seven controls passing.
All 523 saved source bindings matched before publication. No live request
or service deployment was involved in this qualification.

The Responses metered fallback now uses the same tool-completion and remaining
token-budget checks as ordinary responses, preserving incomplete status when
the chunk limit is reached. All 31 affected response/cancellation checks pass;
the parent reproduced six failures with two passing controls.

Browser launch now separates proxy credentials from the server URL, preserving
decoded credentials, IPv6 endpoints and account/global route precedence.
Eight inert proxy-launch tests and five existing browser tests pass; the parent
failed six proxy methods with two controls passing. Both fixes are integrated
on `feat/native-browser-handoff-20261026`, with all 519 saved source bindings
verified before publication. These checks make no live-request, memory-saving
or deployment claim.

Historical tool calls now receive an inferred namespace only when every
matching declaration agrees. Default-namespace collisions and ambiguous
names remain unchanged. The exact 4682a1a source is integrated on
`feat/native-browser-handoff-20261026`; all five focused cases pass after
the parent reproduced both ambiguity failures with three controls passing.

Responses JSON and SSE replies now reject malformed response/output/usage
containers and invalid terminal status before emitting items. Explicit SSE
failures cannot be hidden by a completion. All 44 affected checks pass,
including 13 new cases; the parent reproduced eight failures with five new
controls passing. Valid empty output, optional usage/status and the existing
incomplete-body contract are preserved. The complete 31e73b4 stack is integrated
on the same experimental branch; live acceptance remains pending.

An upstream 401 now removes the rejected AI bearer from the shared session
cache before the existing single refresh retry. A newer cached token, web
tokens and other sessions remain intact. All six inert retry cases pass;
the previous behavior reproduced four failures with two controls passing.
The 59b197d delta is integrated without replacing the newer inference parser.
This corrects local cache behavior and does not establish live acceptance.

Responses streaming now joins continued text before closing the message,
preserves its final fragment status and uses consistent message IDs in events
and the terminal snapshot. All 12 core conversion checks and both actual-chain
cases pass. The old composer reproduced six core and two chain failures;
its existing namespace control passed. The exact cf6d5c6 source is integrated
on the experimental branch, with content-part fidelity qualified above.

Messages fallback now requires closed content and `message_stop` before
publishing output, rejects malformed tool input, preserves retry token limits,
and retains `max_tokens` when the final text chunk is incomplete. The integrated
API suite passed 97 checks; all 12 affected fallback cases then passed with the
retry-limit follow-up. The 13 unchanged parser cases retain their earlier pass.

SQLite startup now persists missing legacy device IDs once in a transaction,
preserving explicit IDs and existing JSON bytes. All 29 store checks pass,
including nine new stability and rollback cases. These changes are integrated
on `feat/native-browser-handoff-20261026` and remain undeployed.

The native minter now connects to the supplied-token browser sender on
`feat/native-browser-handoff-20261026`. It preserves the request's actual
minted device, account and session, and joins browser cleanup. Browser warm-up
now follows history migration and ledger startup, before request admission.
The final startup/credential selection passes 31 tests, the storage selection
passes 20, and the real Rust/Python/browser pipe passes response delivery and
cancellation with no remaining descendants. The feature-enabled CLI builds.
Earlier transport, authorization, service, Python and JavaScript results are
retained for unchanged paths. See the
[handoff and qualification](../reference/analysis/native-browser-handoff-20261026.md).

This milestone also integrates private session-directory checks, nonblocking
regular-file checks, native 16 KiB session-file bounds and correct propagation
of SQLite account-lookup errors. The native browser handoff is undeployed.
Live acceptance, total memory savings, upstream streaming and distinct
per-account TLS fingerprints remain unverified or unfinished.

The native service now retains account HTTP clients on
`feat/retain-native-account-transports-20261026`. Successful requests reuse
their own account generation; session, proxy or limit changes require a new
one. The cache holds at most ten accounts, protects active leases and expires
idle entries after 15 seconds. All 99 focused account, serving and lifecycle
checks pass. Shared shutdown now retains cleanup and its failure across
concurrent callers; reuse waits for device cleanup and the final outcome.
This is undeployed, with memory savings and live acceptance still unmeasured.
Distinct TLS fingerprints remain unimplemented. The next delivery milestone
is controlled live acceptance of the handoff described above. See the
[retention record](../reference/analysis/native-account-retention-20261026.md).

The integrated response path now has a direct local comparison against the
previous full-body host. Across three trials per mode, median sampled peak
PSS fell from 674.653 to 638.521 MiB (5.356%). Separate controls without
sampling took 1345.842 versus 1339.182 ms, with overlapping ranges and no
material latency improvement. All twelve runs preserved the body and joined
cleanup. This is a local response-handling result, not live browserless
acceptance. The roughly 484 MiB pre-request footprint remains. See the
[host measurements](../reference/analysis/browser-response-host-measure-20261026.md).

The host now uses acknowledged response chunks on
`fix/browser-response-bounds-20261026`. All 23 focused host tests and four
package/import tests pass. The actual host path also passes the local browser
fixture: five lost-reply cases preserve the complete body without repeating
HTTP requests, and cancellation retains account ownership through cleanup.
The package includes the new reader, poll and receiver. This milestone is
undeployed. Its functional checks do not establish live-minter acceptance. See the
[host integration result](../reference/analysis/browser-response-host-20261026.md).

The bounded response queue experiment passes thirteen local browser runs on
`fix/browser-response-bounds-20261026`. For a synthetic 8 MiB response,
median sampled peak PSS fell from 664.552 to 651.724 MiB (1.9%); median
request-to-body time without a sampler fell from 1121.7 to 799.7 ms (28.7%).
There were three trials per mode for each measurement. Twelve Node cases
also pass after correcting an absent-body cancellation error; the browser
measurements retain their original source identity. This is undeployed.
A separate acknowledged-delivery prototype now passes a real-browser local
fixture: five injected RPC-loss cases retain the complete body without another
HTTP request; capacity, cancellation, oversize refusal and process cleanup pass.
That prototype established no new performance result. Its subsequent host
integration and account-owned RPC qualification are recorded above.
The minter/browser live connection remains unverified. See the
[queue measurements](../reference/analysis/browser-response-queue-20261026.md)
and [acknowledgement result](../reference/analysis/browser-response-ack-20261026.md).

The earlier nine fresh-browser response-transfer trials pass on
`fix/browser-response-bounds-20261026`. For a completed 8 MiB local response,
128K-unit chunks reduced median transfer time from 763 ms to 434 ms, but
increased post-transfer memory. Neither chunk size established a memory win.
These measurements remain bound to their historical sources. See the
[comparison and limits](../reference/analysis/browser-response-transfer-20261026.md).

The `fix/browser-response-bounds-20261026` branch now limits browser AI
responses to 8 MiB and keeps cancellation owned until the fetch settles.
The real-browser local HTTPS fixture also passes exact-limit delivery,
oversize refusal, cancellation and full process cleanup. Its buffered-response
snapshot used 578.818 MiB PSS including the harness and driver; this is not a
production memory-saving result and includes no SDK or real-account request.
All 17 focused Python tests pass, including 11 JavaScript fetch scenarios.
The oversize regression reproduces on the previous source. This is locally
qualified and undeployed; the native-minter/lightweight-browser connection
and its live acceptance remain unfinished. See the
[change and evidence](../reference/analysis/browser-response-bounds-20261026.md).

The `test/native-tls-account-isolation-20261026` branch passes three local
TLS tests: separate account connections and credentials, forced reconnection,
and required certificate verification. Both TLS 1.2 and TLS 1.3 were tested.
Neither resumed a session, so working session-cache isolation remains unproved.
That earlier milestone changed neither serving lifetime nor TLS backend.
Account transport retention is now implemented above; the missing browser
handoff remains separate.
See the [result and limits](../reference/analysis/native-tls-isolation-20261026.md).

The `measure/shared-context-isolation-20261026` experiment now passes its
local storage, HTTPS connection, concurrent-stream and cleanup checks.
At the two-account snapshot, one browser used 551.233 MiB PSS versus
772.981 MiB for two browsers, including the harness and driver. This single
fixture comparison does not establish production savings, distinct browser
fingerprints or live acceptance. No serving path changed. See the
[recorded result and next step](../reference/analysis/shared-context-isolation-20261026.md).

## Implemented

- Separate browser and native applications in one Cargo workspace.
- Native code grouped by account, authorization, request, transport and worker.
- Browser, native and shared Python runtime packages with explicit entry points.
- Durable account history, exclusive profile ownership and joined shutdown.
- Complete archives with verification, staging and compatible-package rollback.
- A native ledger process that retains its session owner until the child exits.
- Native service state and admission without a browser bridge or browser router.
- Standalone native startup with owned sessions, ledger and a verified worker.
- An explicit legacy-history importer with atomic journal publication.
- A separate expiry transition for legacy profiles with no complete export.
- Explicit native startup integration for that first migration, before admission.
- Native Responses and Messages SSE through the existing request owner.

The retired host patchers, combined-mode checkers and their private tests are
removed. Current tests stay with the behavior they exercise.

## Validation

The connected worker now has a measured memory baseline. One isolated
synthetic operation sampled 118.153 MiB PSS for the JavaScript worker and
121.395 MiB for the Rust owner plus worker. It completed in 2.225 seconds
with joined cleanup; three accounting tests pass. These are sampled lower
bounds, not live inference measurements or demonstrated savings. The
remaining SDK/JSDOM process accounts for most of the observed footprint.
See [the measurement and limits](../reference/analysis/native-worker-memory.md).

Actual SharedWorker and network receiver results now reach Rust, whose
reductions supply all five supported fields to the encoder. Their duplicate
SDK reductions are removed; the SDK retains the genuine network timing path.
A fresh comparison matches all 765 positions, 5,412 serialized bytes and the
original 5,297-byte output, with eleven damage/mismatch checks passing.
Eighteen JavaScript and 91 Rust checks pass. After removing the network
reductions, seven affected JavaScript and five Rust checks pass again, and
the production loader completes a synthetic operation in 2.02 seconds with
the worker joined and absent. Collection still uses SDK/JSDOM; live acceptance
and request memory savings remain unverified. This timing is not a speedup claim.
See [the contract and evidence](../reference/analysis/native-collection-handoff.md#qualification).

The fresh collection frame now passes a comparison through the production
Rust decoder and encoder. All 765 positions reproduce the same operation's
5,411 serialized bytes and 5,297-byte output at its recorded clock. Seven
damage/mismatch checks pass, and the passing capture has confirmed child
exit and no remaining descendants. An earlier outer cleanup-check failure
is retained separately. The existing snapshot checker also accepts the new
capture format. See [the evidence and limits](../reference/analysis/native-collection-handoff.md#fresh-frame-comparison).

Fresh SDK collection now reaches the native encoder through the private
worker pipe. One isolated synthetic operation completed in 1.94 seconds,
with worker cleanup joined and its output discarded. Fourteen JavaScript
checks and 89 focused Rust checks pass. Collection still uses
SDK/JSDOM; live acceptance and request memory savings remain unverified.
See [the handoff and qualification](../reference/analysis/native-collection-handoff.md).

The native encoder now matches one isolated SDK operation using all 765
actual input positions captured before serialization. It reproduces 5,412
serialized bytes and the 5,297-byte final output, including native rebuilding
of the five supported collector fields. Three capture checks also pass.
This establishes the saved direct-input comparison. The new serving handoff
uses SDK collection with native encoding; complete native collection is next.
See [the observation and limits](../reference/analysis/castle-v3-sdk-snapshot.md).

Shared-worker cleanup now waits for actual exit even when stopping fails.
Seven inert checks pass on Node 24.13.0; the three stop-outcome cases reproduce
premature cleanup completion on the previous source. This is adapter cleanup
qualification only. No SDK operation or deployment ran for this change.

Native network sorting now supplies an explicit-locale comparator instead of
requiring JavaScript to precompute the order. Four boundary tests, 133,128
ordering pairs across 18 profiles and 30 complete record comparisons pass on
Node 20.20.0 and the CI runtime Node 24.13.0. Lone surrogates and unknown profiles
are refused. This is collection support, not a completed collector or live
replacement. See [the result and limits](../reference/analysis/native-network-collation.md).

The native network assembler now also exposes the explicit unavailable
record. Its complete serialized output matches the pinned SDK's `xP()`
helper and differs from a normal empty observation. All 32 record comparisons
and the non-scalar refusal check pass in three isolated tests. The existing
CI comparison includes this case. No new SDK startup, network collection or
live request was performed; runtime availability detection remains external.
The saved synthetic SDK capture also passes with its entire unavailable
network record rebuilt natively before applying the consumers. Its 5,412
serialized bytes and 5,297-byte final output remain identical. One native
test passed; the SDK capture was reused without starting another operation.

The native collector-result integration writes SharedWorker position 242
and all four network positions: 80, 469, 473 and 659, in source order.
The related-address and protocol assignments pass 202 original-helper
comparisons; 14 network-assignment comparisons preserve earlier writes on later
failure. The existing 101 hash, 57 address and eight assignment comparisons
also pass. These and refusal/preservation checks run as 25 native tests.
`apply_network_result` now performs the four ordered assignments from one
consumed observation, without rebuilding or cloning its record. It refuses
missing destinations and retains the completed prefix on a later failure.
Reusing the saved synthetic SDK capture, a further native test clears and
rebuilds all five positions in source order, using the native unavailable
network record. Its complete 5,412-byte
serialization and final 5,297-byte output match the same recorded operation.
No new SDK run was needed. Generic accessors and non-string field coercion,
native collection for other fields and live acceptance remain unverified.
See the [source and result note](../reference/analysis/castle-v3-collected-inputs.md).

The [isolated SDK snapshot comparison](../reference/analysis/castle-v3-sdk-snapshot.md)
now passes on one actual instrumented SDK operation with synthetic context.
All 765 positions (787 total values) round-trip to the same 5,411 serialized
bytes. Native final encoding matches the SDK's 5,293-byte output by SHA-256
and length, using the same recorded timestamp. The initial launcher omitted
the syscall filter and correctly failed before SDK evaluation; the corrected
launcher reuses the production filter and passes its denial checks. The
runtime guard is unchanged. Both runs joined, with no account or upstream
request. This is post-collection parity, not a browser or RAM-saving result.
Next bind the actual collector semantics and ordering to the native inputs.

Four native Castle client primitives now match the pinned SDK's unsigned,
signed and fixed-width integer writers and its UTF-16 string writer. Four
direct differential tests compare 12,630 inputs: 4,195 unsigned, 4,219 signed,
4,195 fixed-width and 21 string inputs. They cover integer-width boundaries,
deterministic samples, repeated calls and appending without changing preceding
bytes. Strings include length-prefix boundaries, embedded NULs, surrogate
pairs and unpaired surrogates. The reference executes only source-exact helper
functions and verified aliases from SDK
SHA256 `3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71`.
The isolated run has no network, account data or full SDK execution. Its first
attempt lacked the compiler's `cc` symlink target; adding that toolchain path
to the isolated view allowed the unchanged source comparison to pass.

The typed Rust serializer built on these primitives passes 528 complete-array
comparisons against the original serializer and its source-exact helpers.
The cases cover null/undefined, booleans, numeric boundaries, nonfinite values,
negative zero, float bytes, canonical Base64, rejected Base64 padding, Latin-1,
UTF-16, array-length boundaries and nested ordering. Repeated serialization
produces the same bytes. No full SDK startup or live account was involved.

The original explicit JSON fallback remains available for caller-evaluated
custom objects. Native plain-object conversion now also passes 932 complete
array comparisons against the original serializer. These cover property
ordering and duplicate keys, undefined/function/symbol omission, nested arrays
and objects, Unicode escaping, and 867 numeric values including exponent
boundaries and deterministic samples. Failed JSON conversion discards the
whole object result, preserving adjacent values. Custom getters and `toJSON`
evaluation remain caller-owned; this does not implement arbitrary JavaScript.
The formatter is the locked `ryu-js` 1.0.3 dependency. The first native compile
needed an explicit slice type; the corrected implementation passes.
The first reference extraction and generated Rust fixture needed boundary and
type-annotation fixes; both failures are retained. The final comparison passes.

The native compressor now matches the pinned SDK fallback on 71 distinct raw
inputs and 14 complete serialization/compression cases. Both differential tests
pass, including empty input, repeated bytes, varied symbol frequencies, long
inputs, match-length boundaries and the 1,024-byte search-window boundary.
Every reference stream independently decompresses to the original bytes with
zlib. The fallback is deterministic; the SDK's optional platform compressor
may produce different valid bytes and has not been equated with it.

The final envelope now passes 135 complete-output comparisons against the
original function, using synthetic payloads and explicit timestamps. Fourteen
additional cases match the combined serialization, compression and envelope
sequence. A third test rejects timestamps outside JavaScript Date's range.
The comparisons include empty, odd/even and long payloads, repeated calls,
negative times, decimal-width boundaries and both date-range endpoints.
The shared reference extraction changed; its 528 serializer comparisons pass
again. These are isolated offline checks, not full SDK or service execution.

The V3 worker now uses native encoding; collection still uses SDK/JSDOM.
These comparisons make no memory or acceptance claim.
Native CI includes `scripts/check_v3_encoding.py`,
`scripts/check_v3_serialization.py`, `scripts/check_v3_objects.py`,
`scripts/check_v3_worker_result.py`,
`scripts/check_v3_network_result.py`,
`scripts/check_v3_hash.py`,
`scripts/check_v3_network_record.py`,
`scripts/check_v3_network_events.py`,
`scripts/check_v3_collected.py`,
`scripts/check_v3_deflate.py` and `scripts/check_v3_envelope.py`;
hosted execution remains unverified. All prior encoding comparisons also pass
with the shared fixture builder and new native object module.
Next bind the actual collector values and any custom-object conversions to
this native representation. Device collection, complete client interoperability
and measured RAM/latency remain unfinished.

The native `encode_collected` entry point now combines serialization,
fallback compression and final encoding from a supplied complete value array.
It passes 17 comparisons against the original SDK stages, including three
765-position synthetic arrays; a second test preserves invalid-clock refusal.
Both tests pass with zero skips in isolation. This is a complete post-collection
encoder, not a complete collector or a serving integration. The arrays in the
comparison are synthetic and do not establish genuine device observations.

The [stock shared-worker schema](../reference/analysis/castle-v3-shared-worker-schema.md)
is now decoded from the pinned source. Its previously untyped element 10
comes from `userAgentData.uaFullVersion`, with string markers on unavailable
or failed paths. This does not establish a stock object producer there.
Only the pure string decoder ran; worker execution and live selection were
not tested. Its native `shared_worker_version` reducer now passes 100
comparisons against the original `Qon` followed by serialization. These cover
the eight known markers, unknown strings, strict numeric tag `2`, missing
element 10, JavaScript truthiness and selection without reinterpreting the
result as another marker. A second test rejects already-normalized selected
values whose original truthiness is unavailable. Both tests pass, zero skips,
inside the existing 512 MiB isolated harness. Earlier component results are
retained; the shared reference's only existing-code change exports its decoder.

The [field-38 callback trace](../reference/analysis/castle-v3-network-field38.md)
also establishes a string on ordinary stable RTCIceCandidate paths. Empty,
timeout and selected-address values remain distinct; `krn` substitutes a
marker for an empty field before writing `Q[473]`. No stock non-array object
producer was found on this path. The native `network_address` reducer now
passes 57 comparisons, including five original missing-record errors, plus
nine normalized-input refusals. Both tests pass in isolation. It retains the
original distinction between an absent record and an absent field, and assumes
the built-in getter rather than a replaced JavaScript binding. The independent
metrics-hash input trace is complete. Actual device collection and service
acceptance remain unfinished.

The native JSON metrics hash passes 163 comparisons against the original
`UP` helper, including nine refused conversions. The cases cover property
order, duplicate keys, omitted values, nested arrays and objects, Unicode,
numeric formatting and hexadecimal padding. Repeated calls agree. The first
run reached the generated fixture's compiler recursion limit; the corrected
fixture passes with unchanged native implementation.

The network-record assembler now passes 31 complete-record comparisons
against the pinned SDK's original finalizer. These cover field positions,
selection before sorting, candidate/history caps, duplicates, scalar
conversion, Unicode, summary keys and literal separators. A separate test
checks non-scalar refusal. Both tests passed with zero skips in the isolated
512 MiB harness. The resulting sparse record and its embedded metrics hash
match after serialization. The comparison supplies the original runtime's
signature order; it does not implement or validate a native locale comparator.
Candidates and histories enter after event reduction. No service or account
was used for these tests.

Native `NetworkEvents` now implements that preceding reduction from supplied
typed RTC properties, candidate text, SDP and state events. It passes 56
complete event-sequence comparisons against the original source, plus 32
address-class and 13 numeric-port comparisons, in 57 tests with zero skips.
The cases include branch selection, signature collisions, caps after
deduplication, event ordering, fallback syntax, Unicode and nonfinite ports.
The comparison caught and corrected a native trailing-newline classification
error. Two earlier reference-extraction failures are retained separately.
The isolated run uses the existing 512 MiB harness and Node Unicode 16.0.
No new runtime dependency was added. Generic JavaScript getters/coercions
are outside the typed observation input; Unicode tables remain a platform
boundary. Next connect genuine collection inputs and their finalization order.
Real collection, locale collation, serving integration, acceptance and measured
memory/latency remain unfinished.

The dedicated-worker cleanup correction from `759ef0d1` is integrated on
the reorganized `24ebbdb` base. A focused regression fails on the old pool:
it reports a stop failure before the workers have exited. All eight inert
cleanup cases pass with the correction, with no skipped cases, inside an
isolated Node 20.20.0 run. They cover failed and successful stops, pending
exits, discarded stop promises, source preparation and already-exited workers.
The native CI job now runs this selection. No SDK, live account or service
was used; the matching package result is recorded below.

The G3 correction from `88db846` is integrated over `d760f0a`. A later worker handoff refusal now
retains completed minter metadata. Rust retains a validated typed summary and
emits only fixed labels, booleans, bounded counters and finite timings. Early
errors remain minimal; malformed reports remain generic worker failures.

Failure-only `token_pipe_write_attempted` marks entry into the token writer;
`token_pipe_write_completed` means it returned successfully. A failed writer
may have written partial bytes. Neither flag means an age-qualified
`token_handoff`, parent receipt, acceptance or joined cleanup. Unmeasured or
invalid handoff age is null. The success schema and parent guards are unchanged.

The old JavaScript entrypoint fails the report-retention regression. Its
replacement passes all eight handoff cases, and the Rust parser passes all
22 failure cases. Twelve additional Rust checks pass for HTTP/SSE behavior,
request correlation, cancellation and refusal without a device. No selected
case was ignored. The native CI job includes the JavaScript handoff selection.

Worker failures and request completion now share an internally generated
per-request UUID. An isolated regression checks actual emitted records from
two synthetic requests through the same account: each failure pairs with its
own completion and contains no account identifier. The checks use inert
workers and synthetic state, not the vendor SDK or a live account.
The earlier eight pool-cleanup passes remain valid and were not repeated.
The matching optimized package is now built and checked as recorded below.
The historical HTTP 403 still has no established policy cause.

Native requests now retain one `offline.request.finished` diagnostic after
their owned cleanup settles, even if the JSON or SSE consumer disconnects.
It uses the same safe outcome fields and generated request ID as
`offline.complete`, without claiming delivery or parent-task join. Both
recorders contain logging panics so diagnostics cannot change the outcome.
All seven isolated disconnect/logging cases pass. The old implementation
fails the dropped-JSON case with zero records instead of one. The 41 service
cases and worker-correlation case passed against unchanged recorder sources.
The corrected qualification rebuilt the native test crate after identifying
an old baseline executable reused from a shared Cargo target directory.
These are synthetic checks; the installed service was not changed.

The optimized native release from `04a5fc2` includes the cleanup and diagnostic
changes and passes all eight package lifecycle checks using its bundled worker.
The complete archive is 50,420,493 bytes, with SHA256
`1b2ba1c45db56d5ff1d1b11c458d644ee210295676b44347bf37b7c2df80b8ad`.
Its executable matches the release build, and its worker entrypoint and pool
match the qualified source. The checks cover archive/runtime verification,
startup, invalid requests, exclusive profile ownership, SIGTERM, SIGINT and
profile reuse. Build plus packaging and checks took 72.984 seconds with cached
dependencies. The isolated unit exited and released its execution lock.
No SDK, live account or installed service was used. This is a local package
result. GitHub Actions run `40215277433` ended in `startup_failure` before any
job ran; the previously recorded hosted billing restriction is unresolved.
Next: bind the remaining native acceptance observations to this package and
its request IDs. Existing-account ownership and real delivery remain unverified.

The previous optimized native release from `3c5a999` also passed all eight
package lifecycle checks using its bundled worker configuration. The
complete archive is 50,406,919 bytes, with SHA256
`e780b8d80709f0c064c53cefa90edc0c010a4caa2d9c0d351c57db421eaf109e`.
Its executable matches the release build. These are artifact sizes, not
runtime memory measurements. No SDK or live account was exercised by these
checks. Existing-account migration and native upstream delivery remain open.

Open browser streams can now wait for local account capacity before generated
output. The cumulative wait is bounded to 600 seconds, releases model and
resident permits while sleeping, preserves real cooldowns and cancels on
disconnect. Initial HTTP admission, explicit accounts, prior upstream refusals
and cleanup rules keep their existing policy.

All five source deltas match the qualified `bc47d3d` browser repair. The
reorganized branch passes all 236 server tests with zero ignored; the narrow
repair branch passes 237. Capacity recovery, heartbeat, disconnect and the
full virtual-time budget are covered. The narrow browser package was activated
by an authorized restart at 16:18:57 UTC. Its executable and health PID match;
through 16:21:18 it recorded 152 terminal reply chains and no chain errors.
This branch remains undeployed, and the initial traffic window does not prove
every affected T3 continuation recovered or that a capacity wait was exercised.

The current `c78eeeb` native development archive includes the final-fragment
status correction and latest shared server fixes. Its executable matches the
newly built source, and all eight package lifecycle checks pass with the
unchanged bundled worker configuration. Archive SHA256 is
`605b9ee7926c33c7c09b2944f6174b45a4e6649052f36620b711955b2c57ff31`.
No SDK or live account was used. This closes the gap between current source
and the older packaged native executable; real native delivery is unverified.

The latest browser correction also covers account cooldown after opening a
stream and a replacement account refusing admission while another peer is
ready. Both defects were reproduced before their fixes on the narrow repair
branch, where all 234 server checks pass. The equivalent changes here pass
all 233 server checks, including finite selection and unknown-cleanup refusal.
Explicit account choices, real cooldowns and inference retry limits remain
unchanged. This branch is not deployed; these checks do not prove that all
reported T3 continuation failures are resolved.

A native request regression reproduced completed continuation text retaining
the first fragment's incomplete item status. The shared merge now takes the
latest supplied status while retaining the first identity and combined text.
All 44 native request checks and 15 browser Responses chain checks pass,
including the existing cooldown-continuation case; the native build passes.
This change is not deployed and does not establish live cooldown recovery.

Native streaming now supports explicitly bounded text continuation, with a
separate caller-supplied authorization context for each further chunk. Every
chunk requests fresh credentials through the existing device/rate owner.
The complete-JSON endpoint remains one-attempt, and refusals are not retried.
A regression reproduced output usage reporting only the last chunk; the
corrected response reports cumulative output and reasoning counts.
All 131 selected native request, authorization and service checks pass,
including eight new continuation cases. These cover both formats, context
binding, fresh authorization, cancellation, later refusal, attempt bounds,
incomplete output and withholding unaccepted HTTP output. The native build
passes. This is synthetic qualification; live native delivery is unverified.
The clean `f70e54e` native development archive also passes all eight package
lifecycle checks with the unchanged bundled worker. Its executable matches
the qualified build. The package is not installed in the live browser service.

The existing temporary browser archive checks are now consolidated in
`scripts/smoke_browser_package.py` and wired into the browser CI job. The
clean `4243da2` development archive passes all nine lifecycle checks, including
an actual request for the bundled UI, profile exclusion, both shutdown signals
and later profile reuse. Its executable matches the qualified browser build.
This uses empty temporary accounts and runs no browser, SDK or inference.
The archive is retained for development, not installed in the live service.

Both browser cooldown repairs are integrated into the reorganized application.
A queued unbound request can release a newly cooled account and select an
eligible peer before sending output. A partly delivered reply can wait for
the reported local cooldown on the same account and retry only the unfinished
chunk. Waiting has a separate cumulative ten-minute limit; cancellation,
delivered output, usage and transport-failure timing are preserved.
All 231 current browser-server cases and 27 selected API cases pass, with no
ignored cases. The browser development build passes. These are isolated
synthetic checks. The user's 2026-10-25 15:23 UTC restart activated the separate
narrow repair, confirmed by the running executable's hash. The recorded traffic
window through 15:37 UTC had 857 successful terminal chains and 19 local
cooldown/account reselections. This does not prove every T3 thread recovered or
that the mid-reply wait was exercised. Subsequent T3 errors at 15:40, 15:43 and
15:48 confirm the cooldown loop is still unresolved. This rewrite is not
deployed, and no account cooldown or history was reset.

Native startup now rejects unknown selected accounts before starting legacy
migration or the ledger process. Previously it could complete the ten-minute
expiry first and only then reject the selection. A regression reproduces that
ordering defect. All 11 focused startup cases pass with the correction,
including a mixed valid/invalid selection and an account removed during the
migration wait. The post-wait check runs before listener admission. These are
synthetic profile tests; the native development build also passes. No live
account or service has been migrated.
The clean `96a79c3` development archive passes all eight package lifecycle
checks with the unchanged bundled worker: archive/runtime verification,
startup, invalid-request refusal, profile exclusion, both shutdown signals
and later profile reuse. The archive's executable matches the qualified build.
This package is not deployed and these checks do not run the SDK.

The native `/api/offline/stream` endpoint now sends opening frames, heartbeats
and complete output items through the same bounded request as the JSON
endpoint. Terminal success waits for account and device cleanup. Disconnects
cancel that request while its existing owners finish cleanup. Incomplete tool
output and unaccepted HTTP metadata cannot publish generated items.

The native build passes. There are 74 passing focused service/request cases:
70 retained unchanged results and four affected cases rerun after the final
metadata check. Eight new cases cover both stream formats, response identity,
output before cleanup, failure instead of false success, disconnects before
and after body polling, buffered upstream timing and rejected output. These
use synthetic state and fixture-owned loopback, with no SDK or live account.
Upstream remains buffered and requests remain limited to one attempt. This
does not establish live streaming latency or native upstream acceptance.
The complete `3205333` development archive also passes all eight relocated
package lifecycle checks with its bundled worker, including both shutdown
signals and subsequent profile reuse. It is retained for development and is
not installed in the running browser service.

The reorganized branch now includes the follow-up browser admission repair.
All 139 core and 229 server cases pass on this composition. A matching Busy
close receipt retains the occupied browser slot without declaring a new
unfinished close. Completed status checks followed by a local account fence
also retain confirmed cleanup. Unknown cleanup and mismatched identities keep
their restrictions. This is source qualification, not a new service deployment
or proof that every historical lockout trigger has been resolved.

Native first-migration startup passes nine Rust startup cases and 15 native
Python cases. The native debug executable builds. Coverage includes holding
the profile while no listener exists, completed publication, cancellation,
failed migration, abandoned startup, rejected runtime proof, and later starts
that do not repeat expiry. Python checks preserve device identities, request
charges and unknown daily reserve restrictions. A missing primary can become
a new durable choice from the saved inventory; a known cooled primary still
refuses. All state is synthetic, with no SDK or live migration.
The complete `1b9a7fe` development archive also passes its eight lifecycle
checks: archive/runtime validation, startup, invalid-request refusal, profile
exclusion, SIGTERM, Ctrl-C and profile reuse. It is not installed in the running
service. Real browserless generation remains unverified.
The same package also passes all eight lifecycle checks with the original
V3 worker configuration. The portable worker builder now carries all 1,259
original inputs: 1,258 are byte-identical, and only the location declarations
changed. Node, SDK assets, support modules and npm dependencies are bundled.
Three Rust path-resolution cases and seven package/staging cases pass; the
native executable rebuilds and the packaged software pin checks pass.
The complete `fe306ec` development archive passes all eight lifecycle checks
after staging elsewhere, using its default bundled worker. Startup, profile
exclusion, invalid-request refusal, both signal shutdowns and profile reuse
pass with synthetic state. All six pinned public asset downloads also match.
No SDK execution or upstream acceptance is established by these results.

The portable V3 worker has now also executed the unchanged real SDK with
synthetic context, inside a private network/PID namespace with homes hidden,
no new privileges and a 128 MiB cgroup limit. It generated one token, handed
it to the Rust owner, joined the worker and discarded the token. The run took
2.35 seconds, including 544 ms inside the SDK method. No upstream request or
real account was involved, so live native acceptance remains unverified.
The existing worker measurement is now a normal Cargo example using the same
configuration loader as the native service, with its duplicate parser removed.

The expiry, importer, journal, rate and runtime-owner suites pass 41 focused
Python cases. Thirteen expiry cases cover actual deadline boundaries, unknown
daily reserve use, account isolation, changed owners, clock discontinuities,
existing history, failed writes and inventory changes. These use synthetic
state and clocks. Migration is never selected implicitly, and no live state
has been migrated. The complete-snapshot importer still requires a final
source export. Existing native and browser package results are retained.

The complete browser and native development archives at `2410dac` pass
verification and staging. Browser startup, signal shutdown and profile
exclusion pass. The native archive starts and joins its packaged ledger.

The native admission separation passes all 24 current native service cases and
all 226 remaining browser-server cases. Both debug executables build. The first
compile identified a private method moved across the crate boundary; exposing
that unchanged registration method resolved it.

The prior 288 native results and 110 Python results cover unchanged code.
Four obsolete combined-mode native cases were replaced by four admission
cases; two independent browser health cases were retained. Removed tests do
not count as validation of the new structure. No live delivery is claimed.

The earlier admission correction passed all 228 then-current browser-server cases
and all 34 browser application cases. Its new account-switching regression
fails on the previous implementation and passes with the correction, for
streaming/nonstreaming and consumed/dropped responses. A completed negative
status read keeps the affected account fenced while other accounts can serve.
Failed reads, timeouts and unconfirmed closes retain their refusal behavior.
Health reports 503 when the global operation gate is sealed or unconfirmed.

The 25 October outage was a persistent global admission refusal, not a process
crash. The exact underlying browser check that triggered it was not logged;
the regression establishes a source defect, not that historical trigger.
The narrow repair tracked in PR #3 is running on the original source base; this
rewrite remains undeployed. Health reporting alone does not implement automatic
recovery, which remains unfinished.

Standalone native startup passes 29 service cases, including its admission-aware
health endpoint. Both debug applications build. Six release tests pass, including
refusal of a native archive missing its runtime. Separate child process groups
pass all 64 worker-process and seven ledger-process cases. The `dd2b92d` archive
passes the actual native serve command, profile contention, invalid-request
refusal, SIGTERM, Ctrl-C and profile reuse with synthetic state. It runs no SDK.

## Remaining work

The native CLI now serves ledger-backed V3 with an existing migrated profile.
Real upstream delivery is unverified. The service now watches the ledger's
process exit and stops serving when that owner is lost. All five startup and
seven ledger-process cases pass, including forced helper exit, listener closure
and profile reuse; the native executable rebuilt successfully. The updated
`251eba5` native archive also passes all eight package lifecycle checks.
V2 requires its authenticated SDK proof; no fallback substitutes for it,
and this CLI does not enable V2 serving.

First migration of legacy account history remains unresolved. The working
browser service and its account state have not been changed by this rewrite.
The explicit pre-admission migration stage now holds session ownership and
joins cancellation. First live use still needs the actual previous service
identity and an exclusive idle interval. This does not authorize interrupting
the current service or guessing its history. Native live delivery remains
unverified; the new startup path alone does not establish SDK acceptance.

The requested alternate GitHub account (`diddletime`) and private build fork
are configured. Both available accounts have Actions billing restrictions;
no hosted build has succeeded.

See [architecture](architecture.md), [packages](releases.md), and the
[collected milestone record](history/rearchitecture-progress-20261025.md).
