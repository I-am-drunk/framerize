# Architecture

The explicit `serve_with_browser_control` entry point connects an injected
browser credential peer to native request ownership. One runtime shares one
device owner and permits one opened grant and one issuance. It binds the
account, user, native generation, host generation, lease, device and project
page. Native mint-rate admission and completion still run. The bearer must
cover the request deadline; the peer receives the admitted mint time budget.

Mint RPC settlement leaves the browser control held. `DeviceLease::finish`
finishes that control after request work and transport cleanup, before closing
the underlying device. Unknown cleanup and risk refusal remain conservative.
Startup joins an in-progress peer preparation before peer shutdown, then
shuts down the ledger. Diagnostic requests reject continuation and deadlines
over 180 seconds. HTTP requests retain the existing 60-second limit.

Ordinary `serve` and sender-only `serve_with_browser` keep their credential
source. The new entry point still verifies the pinned worker configuration;
it does not invoke that worker to mint credentials. The concrete private
Python transport, prepared-page launcher, actual egress attestation and live
comparison remain unfinished. The interface alone proves no live acceptance.

The experimental ControlHost connects the browser-mint observer to the current
Host's RPC ownership. One admitted control binds an existing exact device page,
session, host generation, account incarnation and configured proxy revision.
It refuses concurrent account work and blocks ordinary commands and RPCs
through observation and consumer settlement. It never replaces the activity
tracker. Cancellation or uncertainty leaves an admission fence in place.

Physical shutdown uses only the host's close task and its account close tasks.
It first joins the control owner, consumer and tracked RPCs, then calls normal
browser cleanup without clearing the fence. The consumer must report actual
joined cleanup and must not await this same host's close from inside its call.
This dedicated experimental class has no CLI or concrete native transport.
Actual egress attestation and the supervised live comparison remain separate
work. Nonsettling I/O still needs the existing
outer process timeout and confirmed reaping before any recovery.

The inert browser-mint observer in `scripts/experiments/native_browser_handoff`
checks the selected page, context and caller-owned generation predicate. It
compares effective session and device cookies at both the profile and inference
URLs before and after one direct call to the installed wrapper. Inside that
page it reads the current profile, verifies the complete user ID, and checks
page, device and wrapper identity around the call. Non-200 profile bodies are
cancelled and awaited; bounded profile readers release their locks on failure.
It never calls Host._mint, imports another wrapper or selects another page.

The observer is not a live adapter. Its caller owns RPC deadlines, cancellation,
account-generation authority and joined cleanup. The returned time interval
encloses the entire page evaluation, including the profile read; it is not an
isolated SDK duration. Observations do not identify the wrapper's internal SDK
branch or detect transient state changes restored between checks. A matched
comparison still needs the tracked caller and supplied-token sender bound to
one admitted request. This helper grants no timing window or live acceptance.

Dedupe treats a nonempty secondary-project list as recoverable account state.
The dead-account predicate therefore requires no primary or secondary project,
no positive credits and no saved session. Full device identity still controls
clone selection, and survivor management guards still protect applied removals.
Keeping a recoverable record does not make it eligible to serve requests.

KeyringSessionStore refuses keyring's built-in MockCredential before writing,
returning `session keyring backend is not persistent`. That backend stores
the value only in the temporary Entry. Entry-construction errors retain
precedence, and other credential implementations keep their existing writes
and errors. This guard does not enable an operating-system keychain backend.

Offline aggregate-memory diagnostics place the router profile and its worker
in one fresh Linux cgroup. A native supervisor reads `memory.peak` after the
test processes join, preserving peaks between the existing RSS samples.
The counter includes memory charged to the cgroup, such as test buffers and
kernel allocations. Shared pages charged elsewhere are excluded, so this
measurement does not establish total resident memory or per-request heap.

Access-token decoding rejects an empty local or selected relay bearer before
checking expiry. The existing `bad access-token response` and
`relay: no token for user` errors identify their respective paths. An empty
reply cannot enter the token cache, so a later caller may retry the mint.
Nonempty token bytes and the existing session, expiry and fallback rules stay
unchanged; this check does not establish that a token will authenticate.

Project adoption first selects the newest project's string ID, then checks
any root or selected-project teamId against the complete requested team.
Contradictory or malformed metadata returns
`project adoption response has incompatible teamId` before creation.
Missing team metadata stays route-bound. If the first project has no string
ID, the existing creation fallback remains; later projects do not replace it.

Workspace credit scans omit explicit `scope: "user"` metadata entries before
reading their owner IDs. Personal spaces therefore do not produce team-ledger
probes. Every other scope retains the existing ID selection, ordering,
duplicate handling, concurrent joins and error propagation. A profile with
only personal spaces is a complete empty scan, not a failed team probe.

SQLite account and settings operations return the existing mutex error if
the connection lock is poisoned. Account reads and writes no longer unwrap
that error. They do not recover the poisoned connection, clear its state or
retry the operation. Serialization and SQL errors retain their order.

SQLite account upsert requires exactly one affected row before acknowledging
success. A suppressed write returns `stored account write was not applied`.
Serialization, SQL, parameters and earlier errors are unchanged. This checks
the direct statement acknowledgement; it does not read the record back or
roll back arbitrary trigger effects.

After a zero-row account DELETE, the same connection mutex protects a raw
key-existence check. A remaining row returns
`stored account removal was not applied`; an absent key still succeeds.
Nonzero deletes add no read, and malformed account JSON does not prevent
deletion. This is not a general verification of trigger side effects.

Settings writes likewise require one affected row. Other counts return
`stored setting write was not applied` before the caller treats the value
as persisted. Exact keys and values, including whitespace and empty values,
keep their meaning. Existing lock/SQL errors and earlier caller work remain.

File-session IDs retain the empty/separator/NUL checks and Windows drive-prefix
rejection. Accepted IDs use `session-v2-<lowercase SHA-256>.data`; the digest
covers `framerize-session-v2`, NUL and the complete UTF-8 ID. POSIX colon names
remain accepted. The record contains `FRMSESS2`, an eight-byte big-endian ID
length, the exact ID and the cookie. Reads validate that ID before the cookie.
Missing v2 records require reauthentication; the store never reads, adopts or
removes legacy raw-name files. Record labels do not authenticate cookies.
A per-store mutex holds save/delete identity checks through publication and
directory sync. A mismatched or malformed regular record returns
`session record identity conflict`; a poisoned mutation guard returns
`session store mutation unavailable`. Existing validation, I/O and cookie
limit errors retain their order. Nonregular destinations retain OS behavior.

Session saves claim temporary names with create-new semantics. An occupied
name advances the existing sequence; the occupant is never opened or
deleted. Other creation errors return unchanged. The claimed file retains
the existing private permissions, write, sync, rename and cleanup sequence.

Session deletion synchronizes the directory after unlink or an already
absent file. A retry therefore completes the directory-sync step instead
of treating NotFound as proof of completion. Validation and other removal
errors return before synchronization; directory-sync errors propagate.

Credit-ledger parsing requires finite available credits and a finite sum of
allocated credits before constructing a snapshot. Available-credit errors
retain precedence. Overflowing allocation totals return the fixed error
`bad credit ledger response: invalid allocatedCredits total`, while finite
negative, zero and fractional totals retain their original meaning.
After both numeric checks, any present root teamId must be a string exactly
equal to the full requested ID. Missing metadata remains accepted. A
contradiction returns `bad credit ledger response: incompatible teamId`
without normalizing either ID or changing the request, totals or counts.

Responses requests without a numeric output limit leave the chain's total
output limit unset. Explicit limits retain their alias precedence and range
validation. Existing chunk, elapsed-time, cancellation and cleanup bounds
still apply. Chat and Messages retain their existing default limits.

The optional JWT relay builds its request with a query parameter for the
cache nonce. Timestamp capture remains before client construction; URL
validation stays deferred in the request builder. Queryless endpoints and
fragments therefore preserve the request path and transmit the nonce in the
query, while headers, timeout, decoding, expiry and fallback stay unchanged.

AuthorizationState owns a weak registry of asynchronous mint locks keyed by
the complete cookie and endpoint. A cache miss acquires its key's lock and
rechecks token expiry and full-session compatibility before minting. Fast
cache hits remain direct, and different keys and runtimes remain independent.
Registry pruning retains locks with live holders or waiters, including across
token invalidation. No standard mutex spans an await, errors are not cached
or shared, and caller cancellation releases its guard without detached work.

Dedupe retains the chosen survivor's full account snapshot with each clone.
During apply, it validates the clone under its management guard, then acquires
the survivor's guard and rereads its snapshot. Both guards remain held through
clone parking and removal. A missing, changed or busy survivor prevents that
removal; storage errors retain their original meaning. Dedupe never parks the
survivor. Earlier removals remain committed, and this is not a transaction
across the sweep or across processes.

Registration checks one account-list snapshot under the existing registration
lock before saving a verified session, claiming grants or publishing the
initial account. Normalized-email rejection has precedence; a different
account with the same full verified Framer user ID then receives a fixed,
payload-free error. Calls without email perform the same list read and
propagate its original error before persistence. Same-account registration
is excluded from the duplicate check, but another colliding peer still
blocks reauthentication. This preserves user-keyed sessions without repairing
existing duplicates or claiming uniqueness across processes.

SQLite account lookup validates the decoded identity against the requested
key. Listing validates raw text keys against decoded canonical UUIDs before
returning the complete account set. Inconsistent or nontext keys return a
fixed identity error; earlier JSON and SQL errors keep their precedence.
Valid alternate UUID spelling inside JSON is accepted, and reads do not
repair or rekey stored records. This preserves the identity boundary used
by the account guards even when stored rows are inconsistent.

The optional S3w manager owns at most two policy generations under the same
AccountRuntime. Production construction, stage, activate and retire methods
are now present, with retained joins and default legacy0 selection. The real
free Responses SSE route attaches the selected pin and invokes the worker
through the existing account driver. The attachment and its real-child
controls use real child processes with fixture Forward bindings. Credentials,
accounts and cleanup remain kernel-owned. Worker-owned output stays retained
until its acceptance frontier and terminal state are recorded, so disposal
cannot return its output credit before acceptance. Encoded wire capacity is
reserved separately from decoded output. The worker path remains optional;
these local controls do not establish native upstream acceptance.
Worker invocations carry the optional total output limit without inventing
a default. HTTP authorization stays in the shared kernel client, including
its per-key mint guard. A combined real-child fixture holds the first loopback
mint reply until two actual Forward futures are Pending. It checks one mint,
both complete responses, settled application owners and a reaped child.
This exercises composition with an inert browser port, not live authentication.
Frame encoders grow buffers under an explicit allocation ceiling and return
a buffer whose capacity equals its length. Data encoding uses the already
reserved wire credit as its ceiling, including the four-byte frame prefix.
The bounded writer permits compiler inlining across crate boundaries. This
keeps the same per-write checks while reducing serialization call overhead.
Message decoding collects typed fields and then validates their membership
in the selected variant. Segment arrays decode directly into bounded bytes,
without an outer generic Content tree. Unknown, duplicate, missing and
wrong-variant fields remain errors, including explicitly null extra fields.
The discriminator may appear anywhere. Existing positional Serde input
remains accepted. Small nested outcome and diagnostic enums still use their
strict derived decoders; this does not bound total process memory.

Free Responses SSE uses an awaited output sink. Policy transfers a validated
chunk to one encoder and waits for its segments to enter the bounded body
queue before receiving the same chunk owner and continuing the request.
Chat, buffered JSON, legacy callbacks and direct entry points remain available.

AccountRuntime owns the output budget. Each stream reserves an encoder lease
and two 16 KiB output slots before headers or Forward dispatch. Slots follow
the final Bytes owner, including polled clones. Cancellation closes staging
before joining serializers; body output closes before waiting for final buffer
disposal. Channel EOF is independent of that disposal wait. Already-sent
Forward work and its cleanup remain separately joined.

A validated terminal upstream success can still update account health after
confirmed client cancellation. Intermediate chunks, invalid results and local
encoding errors cannot earn that success. Guarded health updates preserve a
newer refusal. Successful upstream work does not prove delivery to the client.

SQLite account listing returns the complete decoded list or an error. A row
that fails JSON or account decoding returns the fixed, payload-free message
`stored account could not be decoded`. Registration, administration and peer
claim checks therefore cannot silently treat an unreadable account as absent.
Listing does not delete, repair or rewrite rows. Query and row-read errors
retain their existing behavior, as do migration, lookup and writes.

Login and import clear carried extra workspace scopes before replacing the
saved Framer user identity. Incomplete later workspace discovery can retain
extra scopes only from the same identity. Same-user reauthentication, primary
project selection and the existing publication checks keep their contracts.
When the identity changes without a selected metadata team, registration
also clears the carried primary team and project before saving. A later
admission failure therefore cannot leave the previous user's primary scope
eligible. Successful admission selects and adopts the new user's scope.

Browser lifecycle status includes retained raw RPCs after caller cancellation.
Idle close reports a busy resident while those operations remain owned, and
joins a close already in progress. Disconnected or unconfirmed browsers keep
their cleanup fences; this does not reopen a globally unconfirmed gate.

Isolated measurements can use `scripts/process_phase.py` to sample a test
process's thread count and virtual address space at established phase barriers.
Each reading checks the PID, start tick and boot ID before and after reading.
These single-process point samples complement the existing owned-tree
RSS/PSS/private-memory sampler; they do not measure encoder allocation or peaks.

Workspace scans join every probe and retain the first error in input order.
Public scans reject incomplete results so provisioning preserves saved extra
scopes. Admission can use an observed unclaimed credited team despite another
probe failure; without such a team it returns the probe error before treating
the account as ineligible. Complete-scan ranking and refusals stay unchanged.

The API credit probe requires a finite numeric `availableCredits` value.
A malformed successful reply returns a fixed payload-free error instead of
an exhausted snapshot. Valid zero, negative and fractional balances remain
unchanged, and authentication, HTTP and JSON errors take precedence.

Registration distinguishes admission refusal from profile and storage errors.
Only `NotEligible` triggers an exhausted-state update. The update reads and
validates the current record under the shared account lock and changes only
its state. A missing or changed subject retains the original refusal without
another write. Storage failures propagate their original error. Other admission
errors preserve the initial saved record; earlier session, grant and account
writes remain separate operations.

Registration starts with a claim snapshot containing peer primary and
secondary team IDs. It filters admission and extra-scope project work while
excluding the current account. After usage probes settle, primary admission
holds the shared account-update lock through subject validation, fresh peer
listing, selection and publication. Late claims can select an unclaimed
backup; incomplete observations retain the original probe error. The helper
patches the latest record and replaces its caller only after a successful
save. Network probes run outside the lock.

Final secondary publication uses that same lock to validate the saved subject,
recheck peer claims and filter prepared extras before saving. It preserves
fresh unrelated fields. The expected scope is captured after admission and,
for import, advances only after a successful intermediate project save.
Project and usage requests remain outside the publication lock. Initial
writes, intermediate saves and rotations retain separate contracts.
This does not establish global uniqueness, roll back project
creation or repair older saved records.

The qualified S2b integration moves Responses policy and protocol conversion into
`framer-protocol`, which has no dependency on account, browser or session
types. Core and API re-export the existing conversion and preparation paths.
`ChainConfig` keeps its public fields; its data snapshot becomes `ChainLimits`.
The shared cooldown algorithm now uses the runtime's narrow `ChainControl`.

Each `native_long` invocation constructs an authorized Forward binding with
that invocation's session, browser, account, project, chat and original optional
callbacks. One category snapshot drives both policy step and request envelope.
The runtime still owns the complete authorization, one retry, refund and
cleanup sequence. Its buffered reply carries the actual post-refund queue
lookup value. Request-correlated queue timing remains separate work.
The legacy API, retry decisions, error strings and saturation call points stay
in place. All 266 isolated S2b checks and ten focused integration checks pass.
This establishes the in-process boundary; a separate policy-worker process
and its delivery, memory and latency behavior still require implementation
and acceptance. The current transport continues to use a browser.

Registration clears a carried primary project when metadata changes its
team, before any explicit project override. Credit admission independently
clears the project if ranking selects a different team. Existing caller
logic then adopts or creates the selected team's project. The two boundaries
retain same-team projects and keep their existing persistence and cleanup order.

The S1 ownership extraction factors browser resources
into one `AccountRuntime`. Each `AppState` shares that runtime. Construction
consumes the stores, reads configuration before moving them, and installs the
concurrency gate and operation owner before sharing. The session-store lock
remains held through host shutdown. Test-only shared mock stores are separate
from this production constructor.

Rotation state belongs to the serving policy. The existing Framer client owns
both authorization caches through its transport. Native session mutation can
carry that same client's narrow cookie-cache invalidator, after the existing
generation drain; account invalidation remains separate. Standalone native
authorization is uncached and does not construct a browser client.

Legacy request handling remains in process. The original `71581d5` candidate
passed 351 isolated checks, and its composition with the newer feature source
passed 37 focused checks. Replacing application generations remains later
work. This step does not remove browser transport or establish memory savings.

Before extracting authorized forwarding, S2a records the legacy behavior in
19 new fixtures. All 165 selected new and retained checks pass on `d22d2b5`,
including the separate agents-category launch. The integrated Forward adapter
binds per request invocation, retains optional cancellation semantics,
and preserves independent retry branches, refund linkage and queue lookup.
These fixtures remain part of the later slices' retained controls.

The kernel/application-generation overhaul is being investigated separately
in [kernel updates](kernel-updates.md). Existing ownership guarantees below
remain authoritative while the new boundary is tested incrementally.

The current reorganization is in progress. See [the design](rearchitecture.md)
for decisions, boundaries and the remaining migration work.

The experimental native/browser transition has an explicit recovery helper.
It verifies pinned inputs and service ownership, joins native shutdown, and
checks remaining descendants before selecting a return binary. Before history
migration it returns to the original browser; afterward it preserves the
journal and retains the compatible browser in a persistent service override.
Recovery restores only the saved or transition-fenced pool configuration.
Before recording a compatible return, it requires account-history readiness
and the expected serving process identity. HTTP liveness alone is insufficient.
It does not admit requests, migrate history or launch a canary by itself.

Acceptance records distinguish upstream refusal, request cleanup and recovery.
A refused request may confirm both attempt and request cleanup even when an
aggregate success-shaped check is false. The October 27 native-token/browser
attempt demonstrated that distinction and returned to the history-ready
`c16fd24` browser. It did not establish native-token acceptance, identify a
TLS cause or justify replaying the consumed attempt.

Matched browser and native archives are checked against isolated synthetic
profiles before live acceptance. The browser package must report usable
account history, preserve unknown history on refusal and report failed-host
shutdown. Both packages must exclude a second profile owner and join shutdown;
native diagnostic records must exclude private request fields. These package
checks do not run the SDK or demonstrate upstream delivery.

An explicit `existing-v2` transition mode accepts an existing schema-2 journal.
It records account rows after the original service has stopped and requires
them to remain unchanged through native startup before sending the canary.
It does not repeat legacy expiry. If the attempt ends before any service
switch, recovery restores only its owned admission change and records
`aborted-original-running`, with the original invocation still required.
After a service stop, the normal history-ready browser return remains required.

Browser and native migration carry the same configured reserve policy.
Reserve-limit changes preserve consumed debt; reboot recovery preserves rate
restrictions. Missing-history recovery requires verified service and session
ownership, retains existing device identities, holds devices for 600 seconds
and keeps unknown reserve debt for 24 hours. Only recognized unversioned
schemas are adopted; unknown history is preserved with actionable diagnostics.
See [rate-history recovery](rate-history-recovery.md) for the boundary.

The separate transition controller requires a systemd recovery supervisor
with finite runtime and stop limits. It uses the existing pool configuration
API to exclude idle accounts progressively while preserving active accounts,
then requires all resident closures before stopping the old service. Each
attempted pool change is recorded before dispatch; recovery accepts its exact
before/after configurations and preserves the original pool settings.
The controller records stop intent and joins the original service and its
descendants before installing or loading the native runtime limit. Recovery
recognizes interrupted stops even when no override has been installed yet,
and retains the original invocation check before joining that shutdown.
Shutdown confirmation waits at most ten seconds for systemd state and the
cgroup to settle. It still requires PID zero and no remaining descendants;
deadline failures distinguish service state from remaining processes.
For a first migration, the native successor must publish a fresh, same-service
expiry record before its one permitted canary request. A durable attempt marker prevents replay
after a lost reply.

The live supervisor uses the host process view (`PrivateTmp=no`) to verify
the running service's executable through `/proc`. Its shared execution lock,
recovery command, memory limits and finite deadlines remain enforced.

Direct inference requires an explicit successful completion event before
publishing its result. Failure events return a bounded error containing the
event name, without upstream diagnostic payloads. The Responses fallback
groups SSE data fields at event boundaries before decoding the terminal
response, preserving the complete tool output and usage object.

Direct completion requires a response object and, when supplied, a completed
status. Its output text replaces intermediate deltas, including an explicitly
empty output; absent output preserves compatibility with delta-only replies.
Malformed terminal text fails before a successful result is published.

Paid Responses aggregation validates each terminal before returning JSON.
Its SSE decoder accepts LF, CRLF and lone CR, rejects named failures, and
requires matching typed data for named completion/incomplete events. Invalid
2xx streams become fixed 502 errors; other upstream status and independently
confirmed cleanup are preserved. Valid incomplete output stays incomplete.

Responses metered fallback carries its successful request's actual token
budget and queue timing into the common completion path. Both response modes
apply tool-completeness, continuation and final-status checks before emitting
items. Failed attempts before streaming do not count as stream throughput.

Responses reject an initial total below the payload builder's 16-token
minimum and stop when less than 16 remains. They preserve incomplete output
instead of rounding a continuation beyond the client's remaining allowance.
They also reject a zero chunk limit before dispatch, so an unattempted request
cannot become a fabricated empty completion.
Messages reject zero chunk budgets, chunk counts and explicit total budgets
before dispatch. Positive budgets and omitted total limits retain their path.
Responses JSON and final SSE derive total_tokens from the aggregate input
and output counters while preserving the remaining usage metadata.
The free-completion handler checks its selected u64 token limit before
narrowing to u32. Overflow returns a fixed client error before dispatch;
field precedence, fallback, zero and the default limit are preserved.
Both free and paid compatibility handlers require an object body before
mutation or account lookup. The free Responses route also rejects a non-null
previous_response_id after route selection, because it cannot reconstruct
stored history. Full input history and early direct/paid routing retain their
existing behavior. Messages subtract their u64 usage before narrowing a
remaining budget and check additions before publishing the current chunk;
discarded tool responses do not contribute to delivered output usage.
Buffered Messages validate their envelope and tool content before callbacks;
all token-limited tools use the existing repair policy, regardless of position.
Completed Messages streams share that block validator after tool reconstruction.
They validate each supplied usage and stop-metadata update, preserve cumulative
usage fields, and treat nullable SDK delta fields as no update. Thinking text
accumulates while each opaque signature replaces the previous value. Named
errors are rejected even without data, and LF, CRLF and lone CR delimit frames.
Messages SSE rendering includes final input, output, cache and tool usage;
already-open snapshots cannot retroactively acquire start-only metadata.
Both chain dialects apply their existing refusal policy to each failed attempt
before paid fallback. Retry timing excludes earlier work and local cooldowns,
while the existing delayed idle-window and genuine transport fallbacks remain.
Responses also check their existing 300-second active-time budget before
explicit rate retries, halved transport retries and metered fallback. The
guard excludes local cooldown waits and allows a successful request already
in flight to complete. The internal cooldown retry remains a separate boundary.
CDP authentication previews end at a valid UTF-8 boundary, and malformed
cookie errors omit the supplied credential.

Historical calls without a namespace are repaired only when their bare tool
name identifies one namespace across all declarations. A default-namespace
collision or conflicting declaration prevents inference; existing explicit
namespaces and all other call fields stay intact.

JSON and SSE response bodies share validation of response/output containers,
terminal status and optional usage. Invalid envelopes fail before item
publication. Explicit SSE failure events cannot be masked by completion;
empty output and the existing incomplete response-body contract remain valid.
Each terminal body is checked before retention, including typed output items,
message content and roles, and supplied usage counters. Named failures remain
fatal without data; named terminals require matching typed data. Both parsers
accept LF, CRLF and lone CR. Valid partial chain results retain their status.
After truncation repair, executable tools require valid fields and unique
call IDs across the current chunk and prior accepted output. Validation occurs
before any current-chunk event. Direct legacy text stops at its completion
boundary and rejects malformed deltas; explicit terminal output is authoritative.
Direct, chained and paid Responses also reject non-null terminal response
errors before publication; omitted/null errors remain compatible. The direct
SSE parser treats a bare data field as an empty data field.

Indexed legacy text is assembled by output/content position in a sparse map
and captured at each completion. Indexed output_text.done replaces its part,
including empty text, and prevents later deltas from extending that part.
Mixed nonempty indexed/unindexed text and
invalid coordinates are rejected only when no explicit terminal output is
available. Later deltas cannot extend the captured answer.

The existing 401 retry invalidates the rejected AI bearer in both account and
shared session caches before refreshing. Shared-cache eviction compares the
bearer so a late rejection cannot remove a newer value; web-token entries
and other sessions are outside that eviction.

Cached tokens retain the session that authorized them. Reuse checks both the
current user and cookie, preventing an account ID retained across login/import
from reusing the previous session's bearer. Refresh failure sends no old token.

Browser session-cookie updates serialize per account and retain their owner
through caller cancellation. Each update waits for device initialization and
acknowledged writes before reporting success; another account can progress
independently. New devices inherit the stored target session. A failed update
can have changed a prefix of contexts, so it returns failure and a later seed
retries every context. Existing admission fences still own session replacement
relative to active requests; this is not a transactional browser-cookie write.

Responses continuation describes the current chunk's tail. The composer
consumes the final text chunk before closing its message, retains the latest
fragment status and carries its emitted message IDs into the final snapshot.
Adjacent message items merge consistently with the chain; tool and reasoning
items retain their original identities and fields.
Other typed output items receive whole-item events and occupy their original
position. They separate neighboring messages, keeping later tool indices
consistent with the final output array.

Each native Responses stream retains one composer from its opening frames
through completion or failure. The composer assigns one event sequence and
keeps generated or supplied tool IDs consistent across item, argument/input
and terminal events. The terminal event follows the assembled status,
including response.incomplete. Both server adapters open this composer eagerly and
reuse it for errors. A failed join emits a fixed failure after settlement;
failure never completes pending content successfully. Native cleanup still
must be confirmed before a successful terminal event can be sent.

While waiting, both item-driven Responses adapters use that same composer
to emit `response.in_progress` every eight seconds. These events retain the
response ID and advance the sequence without replacing accumulated output
or closing pending content. Transport comments do not reset the installed
Codex client's event idle timer. A drained, closed event channel is disabled
while the server waits for the producer to settle. Chat and Messages retain
their existing heartbeat formats; paid/direct passthrough is unchanged.

Message assembly preserves typed content parts, refusal events, annotations,
logprobs and metadata. Only adjacent plain-text boundary parts concatenate;
rich parts retain their offsets and order. Chain and stream share this merge.
Chat JSON retains a separate refusal field, streaming preserves refusal/text
order, and assistant history carries both kinds of content into the next request.
Both Chat renderers preserve an incomplete response's content-filter reason.
Chat function tools retain supplied strictness values through conversion and
continuation, including explicit false/null and omitted-field semantics.
Chat structured-output schemas map to Responses text.format with their name,
constraints and optional settings intact on every continuation request.
Malformed schema wrappers fail conversion before dispatch.

An attached CDP browser bridge claims one account before route or browser work.
That claim survives failures, cancellation and disconnect for its lifetime.
Owned-browser modes retain independent account admission. This claim does not
validate the attached browser's login or exclude other bridge instances.
Attached-page selection parses the URL and requires the HTTPS Framer app
origin, with the default port. A hostname, path or query that merely mentions
Framer cannot select a page. Existing first-page and new-tab fallback order
is preserved.
Owned startup uses the same origin check and waits for a matching page within
its existing discovery attempts. Both discovery paths bound WebSocket TCP
connection and upgrade to five seconds; cancellation drops the owning socket.
CDP discovery GET and new-tab PUT share an owned asynchronous HTTP socket.
One five-second deadline covers connection, writing and complete body reads;
return, cancellation and timeout drop the socket. Headers are limited to
16 KiB and bodies to 8 MiB. The decoder accepts complete Content-Length or
EOF-delimited bodies and rejects failed status, conflicting lengths,
truncation and unsupported transfer encoding. It does not support chunking.
Owned CDP startup retains its child until instance publication or joined exit.
Cancellation requests termination and retains the actual wait result across
abandoned shutdown callers. Version probes use the same ownership mechanism
without an account, so their exit cannot clear a browser's uncertainty.
Version output has a five-second deadline. Expiry returns the existing fallback
user agent and retains kill/wait cleanup for shutdown to join; fallback alone
does not prove the child has exited.
Owned startup rejects stale debugging-port removal errors other than a missing
file before launching the version probe or browser. The blocking removal owns
the account route until it finishes, including after caller cancellation, and
returns that guard without a gap. Same-account work and owned-browser shutdown
wait for removal; unrelated accounts remain independent. Slow filesystem work
can therefore delay cleanup. This is a per-bridge lock, not a global profile lock.
Shutdown joins these children and still closes every known instance. Only
confirmed exits clear their account flags; missing exit evidence stays an error.
Owned Unix browsers retain their process-group identity. Abandoned startup
signals the group only while its unreaped leader still owns that identity,
then joins the leader and waits for group absence. Graceful shutdown waits
for that same proof without forcibly killing surviving members. Attach and
version probes never gain browser-group ownership. Group absence does not
prove cleanup of descendants that leave the group.
The CDP device loader creates an identity only when the record is missing.
Read, UTF-8, JSON and identity-field errors preserve the existing bytes and
refuse startup instead of silently replacing the browser identity.
New records are written and synced in a separate candidate before an atomic
hard link publishes them without replacing an existing record. Concurrent
creators read the winning record; the blocking owner retains candidate cleanup
through caller cancellation. Directory and device preparation retain the
account route until completion, so same-account startup, identity rotation and
shutdown cannot overtake the filesystem work. This does not bound filesystem
latency or establish power-loss durability for the containing directory.
Both browser login adapters select session cookies applicable to
`https://api.framer.com`, matching the API identity lookup. Website-only
cookies are excluded; shared-domain and host-only API cookies remain valid.
Existing ordering, error handling and selection among API-applicable cookies
are preserved.

Python pool-page allocation retains its account lock and memory reservation
until allocation and preparation settle. A prepared page abandoned by its
caller is returned to that account's pool before cancellation propagates.
Each borrowed-page scope also tracks its own dispatched browser calls and
joins them before release, including through repeated cancellation. Other
accounts, free pages and unrelated calls retain independent progress.
Dedicated mint-page creation has a separate cancellation path: it joins
creation and closes a late page without starting navigation or SDK warming.
Repeated cancellation cannot abandon that close; a close failure remains an
error. These ownership rules do not introduce operation deadlines or prove
that remote JavaScript has stopped merely because a local call settled.

Device context creation also retains any late result until it is closed.
Device preparation keeps its lock and memory reservation through dispatched
RPC completion, including cancellation during seeding, warming or cleanup.
Repeated cancellation cannot abandon the close. A failed cancelled close
preserves cancellation and marks account cleanup unconfirmed, preventing
ambient mint fallback and new account admission until browser cleanup.
These guarantees concern local RPC ownership, not remote work termination.

Shared Python driver startup keeps one cached task through caller
cancellation. A finishing waiter clears that cache only if it still owns
the same task, so an older failure cannot erase a newer retry. Shutdown
retains and joins the retry before stopping the acquired driver.

Login completion and session import share one registration mutex from the
normalized-email duplicate check through initial account publication. The
guard releases before credit admission and on early return, preserving the
existing session-save and grant ordering. This protects callers sharing the
same in-process policy. It does not enforce database or cross-process
uniqueness or undo partial session/grant work after a later failure.

Credit admission clears the previous primary project in the same account
update when it selects another team. Existing login/import paths then adopt
or create a project in that team. Same-team admission preserves its project.

Dedupe reports shorten device and team identifiers to a valid UTF-8 prefix
of at most eight bytes. Classification retains the complete identifiers;
short or non-ASCII values cannot panic solely while constructing previews.
The device set excludes records classified as dead, so they cannot cause a
later recoverable peer to be removed as a clone. Retained records still
reserve their complete device identity for subsequent clone checks.
The successful extra-workspace provisioning log uses the same bounded prefix.
Short or non-ASCII metadata team IDs therefore cannot panic between project
creation and retaining its returned ID. Discovery, adoption and stored IDs
keep their existing behavior.

Both CDP evaluation helpers reject a non-null exceptionDetails field before
returning a value. Reported JavaScript failures return a fixed error without
exception descriptions, so a failed device seed reaches the existing startup
error path. Successful values, undefined results and deadlines stay unchanged.
Each CDP command starts one deadline before sending and reuses it for reply
reads, so socket backpressure consumes the same budget. A timeout bounds the
host's wait; it cannot retract bytes or stop JavaScript already dispatched.
Command IDs are recorded before writing and remain owned after cancellation.
Subsequent commands observe prior terminal replies within the existing
deadline. Owned Browser.close may bypass that wait, but only joined child
and group exit can settle commands without replies. Attach shutdown observes
pending commands before disconnecting and retains the socket/state if that
observation is cancelled. Lost replies remain explicit cleanup uncertainty;
startup and mint error handling preserve that signal instead of claiming
completion or starting another mint attempt.

Browser launch converts authenticated proxy URLs into separate server,
username and password options before passing them to the browser driver.
The conversion preserves decoded credentials and endpoint identity while
retaining the account-specific route and existing global fallback.
Chrome CDP registration rejects embedded proxy credentials before changing
account routes, because Chrome ignores credentials in manual proxy settings.
Unauthenticated settings remain supported; this adds no CDP authentication.

Messages fallback validates message and content completion before publishing
blocks. A malformed or incomplete tool call fails the attempt. Retries retain
the caller's remaining output budget, and text continuation retains the final
provider stop reason when the allowed chunk count is exhausted.
Messages citation deltas retain their order and opaque fields. A shared merge
helper joins only adjacent plain text; cited and other rich blocks keep their
boundaries and metadata through assembly, continuation and both SSE renderers.
The API Responses chain checks accumulated output and combined token counts
before publishing a chunk, rejecting overflow without exposing that chunk.

SQLite startup assigns the existing default device ID once to valid legacy
records that lack it. One transaction persists these assignments before the
store becomes usable; explicit IDs and original JSON field bytes are preserved.
Any failed update or commit rolls the migration back and refuses startup.
Migration reads each row's raw SQLite text through checked UTF-8 conversion.
Malformed text and nontext values are left unchanged, with traversal advancing
to the next row. They retain their later lookup/list errors instead of causing
an owned-text conversion panic during startup. This does not repair invalid
records or make them eligible for account admission.

The optional native `browser-transport` feature connects the native minter
to a supplied-token browser sender. Each request carries its actual minted
device, account/session binding and freshness deadline. One browser creates
a separate context per active request and closes it before confirming cleanup.
The sender performs one POST without minting or retrying a refused request.
It returns a bounded complete body through the native response owner.

Native startup verifies inputs and accounts, completes any required history
migration and starts the ledger before preparing the browser. Admission waits
for preparation. The application retains browser shutdown ownership across
preparation failure or cancellation; the service joins its ledger before
releasing the profile. This is an experimental development option. Separate
contexts do not provide distinct TLS fingerprints or prove remote unlinkability.
See the [handoff contract](../reference/analysis/native-browser-handoff-20261026.md).

Browser AI response reads enforce an 8 MiB byte limit and drain a queue
during the fetch. Queued and unacknowledged text share a 131,072 UTF-16-unit
capacity. Each chunk remains replayable until the host validates and
acknowledges it; final acknowledgement removes the job. Python still returns
one complete body and discards partial output on a body failure.
Every start, poll and cancellation uses the account-owned `Host._rpc`.
The receiver shares the request deadline; a timed-out waiter retains the
underlying RPC owner until settlement. The abandoned-job timer keeps cleanup
joinable. Local host and browser tests cover delivery, lost replies, bounds,
ownership and cleanup. See the
[integration result](../reference/analysis/browser-response-host-20261026.md).

A comparison of the actual host implementations on frozen runtimes now
records a modest sampled peak-memory reduction. The separate controls without
sampling show no material latency improvement. Python still assembles the
whole bounded response, and the browser/driver starting footprint remains.
These measurements exclude account startup, SDK collection and live delivery.
See the [host comparison](../reference/analysis/browser-response-host-measure-20261026.md).

The earlier response-transfer experiment compares the then-current full-body
poll with bounded chunks from an already completed body. Python still joins
those chunks into one response. Nine fresh local trials preserve contents
and cleanup, but show no clear memory win. See the
[comparison](../reference/analysis/browser-response-transfer-20261026.md).

A second experiment drained a 131,072 UTF-16-unit queue during the fetch,
with backpressure and the same joined cancellation. Thirteen local browser
runs preserve complete bodies, limits and cleanup. The original reader shows
a modest sampled memory reduction; twelve Node cases pass after a separate
absent-body cancellation correction. Those measurements retain their original
source identity. The measured poll consumed text before receipt confirmation.
A subsequent acknowledged protocol retained each frame and its capacity until
receipt, including final-job removal. Its standalone browser fixture passed
five injected RPC-loss points. That protocol is now integrated above; the
prototype measurements do not establish performance for the integrated host.
Historical comparisons require their recorded source revisions. See the
[queue measurements](../reference/analysis/browser-response-queue-20261026.md)
and [acknowledgement contract](../reference/analysis/browser-response-ack-20261026.md).

The native HTTP transport builds an independent reqwest client per account
generation. The service now leases that generation and retains it after a
successful request with confirmed cleanup. Reuse requires identical account,
user, session, proxy and limits. Active leases prevent idle eviction; the cache
holds at most ten accounts and expires idle entries after 15 seconds.
Reuse is committed only after device cleanup, account-state publication and
the final cancellation check. An unfinished lease closes on drop. Shutdown
callers share the same native closure and its result, including failures.
Mutation barriers still close the actual generations. See the
[retention contract and qualification](../reference/analysis/native-account-retention-20261026.md).
A private builder helper allows local TLS fixtures to trust an ephemeral
certificate without changing production trust or TLS selection. Three earlier
tests verify connection reuse, credential separation and certificate refusal.
See the
[TLS result and limits](../reference/analysis/native-tls-isolation-20261026.md).

The separate shared-context experiment measures two synthetic accounts in
one browser versus two browsers using a local HTTPS server. It verifies
storage and connection ownership while another account streams or closes.
It does not change account launch, generate TLS fingerprints or establish
remote unlinkability. The production browser service still owns a browser
per account. See the [measurement and limits](../reference/analysis/shared-context-isolation-20261026.md).

The experimental V3 worker collects values in a fresh JavaScript SDK instance
for each operation. Its private pipe returns a bounded, typed collection to
Rust, which serializes, compresses and encodes it after the child exits.
The request owner retains both child cleanup and native encoding completion.
V2 keeps its separate token protocol. The normal browser service is unaffected.
See [the handoff contract](../reference/analysis/native-collection-handoff.md).

The separate `worker-smoke` example samples its own memory and that of its
descendants while running one isolated operation. It checks process identity
and membership around each sample and joins the sampler on completion.
This instrumentation is not part of the serving path. See the
[connected worker baseline](../reference/analysis/native-worker-memory.md)
for measured scope, accounting rules and limitations.

The private collection carries the same operation's SharedWorker and network
receiver results. Rust reduces them into position 242 and network positions
80, 469, 473 and 659 before encoding. Those SDK reductions and assignments
are omitted. The SDK retains the actual network elapsed-time sample and its
later conversion at position 755. Collection still uses SDK/JSDOM; this step
does not replace the observation environment.

The isolated snapshot probe observes one original SDK operation and sends
its collected values through the same frame format. A separate checker uses
the production decoder and encoder with that operation's recorded clock.
This comparison does not introduce observation hooks into the serving loader.

Native Castle client encoding lives in `framer-native/src/castle/`. Its typed
serializer selects the pinned Framer V3 SDK's numeric and string representations,
writes array structure and preserves UTF-16 code units, including unpaired
surrogates. It has no browser, JavaScript, network or account dependency.
Nested arrays and JSON objects use explicit traversal stacks. Native objects
carry own enumerable string properties in insertion order. JSON conversion
sorts array-index keys, retains other property order, omits unavailable object
values, and preserves JSON's array-null and UTF-16 escaping rules. The locked
`ryu-js` dependency provides ECMAScript number formatting.
Custom getters and `toJSON` evaluation remain caller-owned: their result can
be supplied as JSON text for its actual position, or as a failed conversion.
The native data model does not execute arbitrary JavaScript or recreate its
object graph, descriptors or prototypes.
`scripts/check_v3_encoding.py` compares the primitives, and
`scripts/check_v3_serialization.py` and `scripts/check_v3_objects.py` compare
complete arrays against the original serializer. The reference extracts only
the required helpers and constant table;
it never loads the SDK module. The experimental serving path uses these encoding
primitives, while collection still requires the SDK and its JavaScript runtime.

`castle::v3::shared_worker_version` reduces a supplied worker result before
serialization. It preserves the SDK's marker translation, strict success
tag and JavaScript truthiness when selecting element 10. Its input is data,
not a worker handle: it does not launch a browser or collect a version.
The selected value must retain its original type; precomputed JSON conversion
cannot establish truthiness and is refused at that position. The offline
comparison in `scripts/check_v3_worker_result.py` runs the original reducer
and serializer on synthetic inputs.

`castle::v3::network_address` reads the supplied network record's field 38
with the SDK's built-in getter semantics. Missing fields use the original
marker, while a missing/null first record remains an error. It has no network
access and does not substitute an address for the caller's observation.

`castle::v3::json_hash` applies the SDK's metrics hash to supplied values.
It reuses native JSON conversion, then performs wrapping 32-bit FNV-1a over
UTF-16 code units and returns eight lowercase hexadecimal digits. Conversion
failure or a top-level undefined value produces no hash. The caller still
owns metric collection, ordering and any custom JavaScript conversion.

`castle::v3::assemble_network_record` packs the collected network snapshot
into the SDK's sparse record and hashes its six-part metrics array. The caller
supplies candidate fields after event reduction, state histories and the
observation runtime's signature comparator. The assembler takes the first
16 candidates before stable sorting, retains eight entries per history,
builds protocol/group summaries and preserves the original field positions.
Its comparator is explicit because JavaScript `localeCompare` has no single
portable byte order. Candidate deduplication, state normalization, callback
timing and collection belong before this boundary. The isolated comparison
uses the original finalizer and compares the serialized complete record.

`collection::NetworkCollator` supplies native signature ordering to that
assembler. Its ICU4X 2.3 dependency uses the existing workspace ICU versions.
The caller supplies an explicit supported locale from the observation runtime.
Selected scalar fields are checked before sorting; lone UTF-16 surrogates are
refused because ICU4X would replace them, changing JavaScript's ordering.
Unsorted histories retain their raw code units, and candidates after the first
16 remain ignored. The original caller-supplied comparator API is preserved.
The [collation comparison](../reference/analysis/native-network-collation.md)
records tested runtime versions and limits. Locale alone does not establish
compatibility with every JavaScript engine. No collector or serving switch
is implied by this pure assembly support.

`unavailable_network_record` separately represents the SDK's no-argument
`xP()` fallback after confirmed network unavailability. It preserves the
empty fields and the fallback's distinct hash. Callers own the availability
decision; this function neither probes the network nor turns collection
errors into a successful empty observation.
The snapshot comparison can require this record to be reconstructed natively
before applying its consumers, then compare the complete encoding against
the same saved SDK operation.

`castle::v3::NetworkEvents` supplies that earlier reduction from an ordered
sequence of typed observations. It normalizes structured RTC properties or
fallback candidate text, classifies addresses and numeric ports, and shares
the first-16 signature set with SDP protocol entries. Each state history
retains its first eight distinct lowercase strings. Consuming the accumulator
creates `NetworkObservation`; the caller supplies both selected addresses.
It never starts RTC, reads interfaces or guesses observation timing. Property
getters and nonstandard JavaScript coercions stay outside the typed boundary.
Unicode lowercasing uses the Rust toolchain's tables, so non-ASCII behavior
also depends on matching the source runtime's Unicode version. The offline
comparison runs original pure candidate, state, SDP and finalizer functions.

The native `castle::v3::deflate` implements the pinned SDK's raw-DEFLATE
fallback: a 1,024-byte matching window, at most 64 candidates per lookup,
and one dynamic-Huffman block. Its stable tree ordering and code-length
encoding follow the source. `scripts/check_v3_deflate.py` checks both raw
inputs and serialized values against the original fallback, then uses zlib
to verify decompression. The SDK may instead use a platform CompressionStream;
matching the fallback does not establish identical platform-compressor bytes.

`castle::v3::envelope` wraps supplied compressed bytes with an explicit
millisecond timestamp. It implements the pinned SDK's timestamp encoding,
payload transformation, four checksums, length fields and final Base64.
The caller supplies the clock; the wrapper has no device, account or network
access. `scripts/check_v3_envelope.py` compares complete outputs with the
original function and checks serialization through final wrapping together.
This is a client encoding component, not device collection or proof of
service acceptance. The experimental native worker uses it after collection.

`castle::v3::encode_collected` is the complete post-collection entry point:
supplied value array, typed serialization, SDK fallback compression, then
final encoding with an explicit timestamp. It preserves supplied positions,
types and undefined entries without creating missing observations. Length
overflow and envelope errors remain explicit. It has no JavaScript runtime.
The SDK populates its input during both configuration and request execution;
this API requires the completed snapshot and does not establish readiness.
Its comparison includes full-size synthetic arrays through all original stages.

A separate [snapshot experiment](../reference/analysis/castle-v3-sdk-snapshot.md)
observes the real SDK's serialized output and encoder timestamp in the
isolated synthetic worker. Its test-only loader leaves the production loader
unchanged. The native check reconstructs normalized values, verifies their
serialized bytes and compares the final output by hash and length. This
checks one post-collection observation, not the preceding collector semantics.

The snapshot experiment now also records the actual array passed to the
serializer. Its schema 3 comparison builds native values from this input,
checks serialization, rebuilds the five supported collector positions, then
compares the final encoding using the same operation's timestamp. This is a
test-only observation boundary. The serving worker now sends its value array
and both receiver results to the native encoder. Other observation
producers still run in the SDK; each replacement needs its own connected
input and ownership boundary.
The network completion timing is separate from the four record fields: its
setup-to-fulfillment elapsed milliseconds feed position 755. It must retain
the actual timing assignment and ordering, not a fabricated duration.

`apply_collector_result` places supplied SharedWorker and network outcomes
in position 242 and network positions 80, 469, 473 and 659. The network
assignments run in that order: related address, hash, address, protocols.
The related address, hash and protocols reuse the envelope's string transform.
`apply_network_result` consumes one observation and applies all four network
assignments in order, without cloning the record. It checks each destination
before conversion and stops on the first error, retaining earlier assignments.
`apply_collector_result` remains available for individual assignments, including
the SharedWorker value. The transformed fields accept original strings and
missing-field markers. JavaScript accessors and truthy non-string coercions
remain outside the typed interface.
The [collector comparison](../reference/analysis/castle-v3-collected-inputs.md)
rebuilds all five positions from the retained synthetic SDK capture. It
does not fill the remaining collection dependencies.

An open, unbound browser stream can wait for local account capacity before
generated output. It retries eligibility within one cumulative 600-second
wait budget without holding model or resident permits while asleep. Stream
heartbeats and disconnect cancellation remain active. Real cooldowns, prior
upstream refusals, explicit-account pinning and cleanup checks are preserved.
This wait does not change initial HTTP admission or inference retry limits.

Browser stream admission can reselect before generated output if its account
cooled after the stream opened. Reading that cooldown does not extend it.
Replacement selection checks each original candidate at most once and may
continue after a confirmed local admission refusal. Unavailable accounts keep
their fences; unknown cleanup, global pauses and internal errors stop the
selection. Explicit account selection and inference retry limits are unchanged.

- `apps/framerize`: the normal browser application and command-line interface.
- `apps/framerize-native`: the experimental standalone native service and tools.
- `crates/framer-native`: native account, authorization, request, transport and worker code.
- `crates/framer-api`: Framer endpoint policy, browser chains and shared payload helpers.
- `crates/framer-core`: domain types, account selection and ownership primitives.
- `crates/framer-server`: normal HTTP routes and browser account admission.
- `crates/framer-host`: browser process integration.
- `crates/framer-store`: state and session storage.
- `runtime/framerize_host`: Python browser host, with account, browser, resource
  and transport modules.
- `runtime/framerize_accounts`: shared journal, rate policy and crash recovery.
- `runtime/framerize_native`: native ledger ownership, independent of the browser host.
- `runtime/framerize_runtime`: process framing and package verification shared by both apps.
- `scripts/package.py` and `scripts/release.py`: build a complete archive and
  verify it into a new release directory.
- `web`: the application interface.

One Cargo workspace and lockfile cover both applications. The default target is
browser-based. Native depends on the shared libraries; those libraries do not
depend on native. Their browser routes do not select a native provider.

Native device admission is a separate `framer-native::account::device_owner`
contract. Its grants and authenticated SDK proofs live with the native account
code. Native serving receives this owner explicitly; the browser bridge no
longer exposes native enrollment, mint or cleanup methods. Request cleanup
still retains the supplied owner until both transport and worker have joined.

The file session store lives in `framer-store/file_sessions.rs`. One process
owns a session directory with an OS file lock for the store's whole lifetime,
matching the journal's profile ownership. Different paths to that directory
do not create independent owners. The lock file stays in place across exits;
kernel lock ownership, not its presence, determines whether startup can proceed.
Session replacement uses a private temporary file and an atomic rename.
On Unix, opening a store rejects a session directory with any group or other
permission bits. This also applies to existing directories, before taking
ownership. The store does not change existing permissions or migrate files.
Session and ownership entries must be regular files. Unix opens are nonblocking
so a FIFO can be rejected through its opened-file metadata without waiting for
a writer, before locking or reading any contents.
Native startup uses the store's optional cookie limit with the same 16 KiB bound
as native credential validation. After the record header and ID, cookie reads
take at most one byte beyond that bound to detect overflow even if a file grows;
oversized saves fail before replacement. The cookie bound excludes the record
header and ID.
The default store constructor has no size limit, preserving other consumers.

Native generations retain the store with their HTTP client. Credential
preparation also retains that client until its worker ends. Closing the store
bridge or dropping a response waiter therefore cannot release profile ownership
while that work is still held. The lock is cooperative: old binaries do not
honor it, and it does not replace the journal's proof of prior-process cleanup.

The old parallel checker and combined-mode startup/routing runners are removed.
Their [validation record](history/parallel-browserless-validation-before-reorganization-20261025.md)
is historical. Browser/shared Python tests live in `runtime/tests`; native
Python tests live in `runtime/tests_native` and run in the native CI job.
Rust tests stay with the libraries whose behavior they cover.

Legacy host source patching, one-use restoration packages and their combined
application/worker fixtures are also removed. Build the browser package with
`scripts/package.py`; its runtime uses the journal and shutdown path described
below. The old setup and measurements remain in the history, with immutable
source links where relevant.

The browser archive carries its matching host and UI. The executable resolves
both from its own package, independently of the working directory. Development
overrides are explicit command-line options. Native archives carry their ledger
runtime and standalone service. See [application packages](releases.md).
Both applications have a package lifecycle command used by their own CI job.
The browser command runs from temporary empty account state and checks the
packaged UI, profile ownership and shutdown without starting a browser.

The native package also carries its V3 worker under `runtime/worker`, including
Node, pinned SDK assets and locked npm dependencies. `bundle.mjs` assembles those
files without running the SDK; package creation checks the complete inventory.
The worker configuration uses paths relative to its own location. Moving an
archive therefore does not bind it to the build machine or current directory.
Startup verifies the packaged runtime and the existing 32 direct worker pins.
Explicit external worker configurations remain supported for development.

`framer-native::worker::from_config` is the shared configuration entry point
for the service and the `worker-smoke` Cargo example. The example runs one
synthetic mint only inside its required network/PID/filesystem isolation and
memory limit, retains cleanup and discards the token. Its success establishes
local SDK execution and handoff, not authenticated upstream delivery.

The dedicated-worker pool retains each stop result until every worker has
exited and every stop has settled. An early stop failure cannot finish pool
cleanup while another worker is still running. A discarded browser-style
`terminate()` result remains visible to the later pool close. This local
cleanup is separate from the Rust owner's joined worker-process receipt.

Shared-worker close also retains both the stop result and the actual exit.
A rejected or throwing stop waits for exit before reporting failure. An exit
does not discard a pending stop result. If exit never arrives, the existing
Rust process deadline still owns termination and joining of the whole worker.

Native startup activates the shared event sink after selected-account
validation, retaining the actual profile path without lossy text conversion.
Log-open errors produce one fixed stderr warning and leave the writer unset
so later events can retry. They never replace an obstructing file or abort
serving. Events lost while the destination is unavailable cannot be recovered.

Each native service request creates an internal random diagnostic ID. Its
credential source passes that ID to the worker minter, and JSON or SSE
completion logging uses the same ID. These IDs contain no account or inbound
request identity. Worker failures retain only validated phase facts: fixed
labels, booleans, bounded counts and finite timings. Pipe-writer entry and
return are distinct from age-qualified handoff and parent process cleanup.
This correlation does not add observations of unreported SDK or server phases.

The request owner emits `offline.request.finished` after its resource cleanup
settles, even when the client has disconnected. `offline.complete` records
the result observed by a waiting HTTP or SSE handler. Both use the request's
diagnostic ID and the same safe outcome fields. Neither event proves remote
receipt or the enclosing operation task's join. A logging failure cannot
change the request result or interrupt its cleanup.

Concurrency-limit storage is selected by application startup. Library gates
are in memory unless supplied a state-file path. Fixtures never inherit the
normal service's learned limits through an implicit home-directory lookup.

The browser host exposes only browser commands. Ordinary account activity and
pending RPC ownership live in `accounts/`; native observation and comparison
adapters are absent from both the Rust bridge and the Python browser package.
Native `LedgerOwner` uses the same journal and rate policy without importing
the browser host. It opens only existing account history and its saved primary
device, serializes mints per account, and commits charges before returning
admission. Grants remain owned until confirmed cleanup. Failed writes and
uncertain cleanup prevent reuse; closing storage cannot mark them clean.
The old native helpers coupled to host restoration and observation are removed.

This owner controls ledger access, not login. Standalone startup owns the
session directory and ledger before accepting requests, verifies the declared
worker files, and requires existing migrated history and selected accounts.
It serves ledger-backed V3 only. It does not supply V2 SDK proofs or replace
the native library's authenticated proof requirements.

The Rust ledger adapter starts `runtime/native.py` explicitly and retains the
session store until that child exits. Its private pipes carry device operations,
not session cookies. Abandoned open or mint-admission replies are cleaned up by
the connection owner; loss of the connection reports uncertainty. Shutdown
requires both a clean ledger reply and the observed child exit. The native CLI
connects this owner to the worker and the loopback complete-JSON endpoint.
SIGTERM and Ctrl-C drain requests, close native sessions and join the ledger.
Ledger and worker processes use their own process groups so terminal signals
reach this coordinator first. Startup and package checks use synthetic state;
they do not establish successful upstream browserless delivery.
The service also watches the ledger's actual process-exit notification.
Unexpected exit stops HTTP admission and enters the same shutdown path;
unconfirmed ledger cleanup produces a failure exit and is never marked clean.

Native session ownership is constructed from a session store and an operation
tracker. It retains both for its lifetime and drains its own tracker during
shutdown. Requests must use that exact pair. The session component does not
receive or mutate browser application state.

Native serving has its own state and admission policy in `framer-native/service`.
It accepts a fixed set of 1 to 64 account identities, with one logical slot for
each selected account. There is no browser status probe, browser eviction or
account-management route in this service. Identity, proxy, capacity, cooldown
and cleanup checks remain in the native admission path. The browser server's
old native control and comparison-reservation entry points are removed.

The host opens one SQLite journal outside the release directory. Account rate
decisions live in `framerize_accounts/rates.py`: request charges and mint reservations
commit before external work. Device choices, cooldowns and reserve usage persist
across browser closure. A cancelled mint retains its own RPC and pacing lock
until the actual operation ends. Confirmed browser closure records clean state.

The legacy process-memory restoration code is removed from the runtime.
Existing profiles without a journal require migration. Interrupted records
require confirmed cleanup before reuse. First migration remains unfinished;
this code is undeployed.

`framerize_accounts.expiry.ExpiryWitness` supports the legacy case with no
exported history. A successor keeps the profile exclusively idle for at least
the old 600-second local rate window. The witness holds the journal lock and
rechecks the service lifetime, clock and saved device inventory before atomic
publication. It records unknown-expired provenance instead of fabricated past
requests. Stable device IDs survive, the unknown former primary stays unset,
and unknown reserve usage remains fully allocated for at least 24 hours.
The native application's explicit `--expire-legacy-from` option wires this
transition before its listener and ledger start. The parent holds session
ownership until the migration helper exits, including cancellation or an
abandoned startup caller. Existing history skips expiry. The transition still
requires the old service to be gone and does not establish upstream acceptance.

Startup validates every selected account in the profile's database before
starting migration. It repeats that validation after expiry, before opening
the listener or ledger. An invalid selection therefore cannot spend the
ten-minute expiry interval, and a selection removed during that interval cannot
start serving. Both checks retain session-directory ownership.

Native admission may assign an unset primary from the durable ready-device
inventory and persist that new selection before granting work. This does not
claim to restore the unknown former primary or create a new device identity.
A known primary's cooldown or exhausted request allowance still refuses work.

The browser application always tracks its requests and background work.
SIGTERM or Ctrl-C stops new HTTP admission, lets accepted responses finish,
and joins detached application work before shutting down the host. The host
joins raw browser calls and records confirmed account closure before exiting.
The Rust bridge waits for that process exit. An unconfirmed close is an error,
not a successful drain. Attached user browsers are disconnected, never closed.
Owned children use separate process groups so foreground terminal interrupts
reach the daemon's shutdown coordinator first. Forced exits still require
recovery and are not equivalent to this graceful path.

The journal records the service lifetime beside each account snapshot. A new
host verifies its parent is the service's main process, delegation is disabled,
and the cgroup contains only that parent and itself. Dirty history can recover
after a different invocation of the same service, or a kernel reboot. Surviving
children, unknown ownership and a different service prevent automatic recovery.
Recovery preserves charges and device identities and applies a fresh cooldown.

Journal schema 2 adds per-record runtime ownership. Schema 1 upgrades without
inventing ownership for old records. Older readers reject schema 2, so package
rollback must select a compatible reader. No caller-provided clean flag or
missing lock file substitutes for process cleanup.

The native/browser canary now has a checked schema-2 browser return archive
from the same source revision. Its release binaries and package lifecycle
checks are complete; deploying it still requires the genuine history transition.

`scripts/release.py activate` updates an already migrated user service. It
checks package and history compatibility before stopping the service, switches
the package symlink atomically, and restores the previous package if startup
fails. Readiness checks match the health endpoint's PID to systemd's main PID
and the selected executable. One persistent update record supports recovery
after interruption; account data is outside that transaction and is never
rolled back. First migration from the legacy service remains separate.

A completed browser-status read can report an unavailable account without
leaving request work unfinished. That account retains its occupied, fenced
slot; other accounts can continue. Failed or mismatched reads, timeouts and
unconfirmed closes still carry unresolved cleanup evidence. `/api/health`
returns 503 while the operation gate is sealed or unconfirmed, retaining the
PID header. This endpoint neither clears the gate nor guarantees upstream
account availability. Reporting failure does not itself restart the service.

A matching Busy close receipt is a refusal to begin closure, not an unfinished
close. It cancels that close reservation while retaining the occupied slot and
any prior uncertainty. A completed status read followed by a local account
fence is likewise a local refusal. Neither path frees capacity or clears
unknown cleanup; other accounts can continue serving. Refused closes record
their outcome and whether the receipt matched the expected browser identity.

Queued browser requests recheck their account after obtaining a model slot.
If an unbound request's account cooled while waiting, it releases that slot
and lease before trying up to two eligible peers. Explicit account choices
stay pinned. Exhausted selection retains the actual cooldown deadline.

After a browser generation has produced output, a local cooldown can wait
on the same account and retry only the unfinished chunk. This permits one
retry per refused attempt and at most 600 seconds of waiting per chain.
Cancellation is polled during the wait. The 300-second generation budget,
chunk sizing and transport-failure timing exclude those waits. Delivered
output, usage and request context are preserved. Both browser request formats
share this logic in `framer-api/chain`; upstream refusals retain their policy.

The native HTTP service offers `/api/offline/complete` and
`/api/offline/stream` over the same explicit request context, account owner
and cleanup lifetime. Both default to one attempt. The stream sends standard
Responses or Messages SSE, opening frames and keepalives. Its output queue
holds at most one bounded response per permitted attempt. Dropping the HTTP body cancels
the original request; registered cleanup continues to hold its resources.
Completion is emitted only after the account and device cleanup finishes.
Failed cleanup produces an error without a successful terminal event.

Both endpoints accept the existing explicit context envelope. The inner
`request.stream` remains `false`; `/api/offline/stream` selects downstream SSE.
Without continuation, the 16 to 512 token limit still applies. The maximum
60-second request deadline applies to the whole request in either mode.
Clients must observe the terminal event, since HTTP 200 only opens the stream.

The stream may opt into bounded text continuation. The caller supplies a
continuation prompt, a total output budget up to 32,768 tokens, and the exact
authorization context for each of at most 15 additional chunks. Only text cut
off at the output limit may continue. Each chunk obtains fresh credentials
through the same account's device owner and rate admission. Refusals are
terminal; neither a new account nor the first chunk's token is substituted.
The original input, accepted output and supplied prompt form the next input.
Reported output and reasoning token counts accumulate across the chain.
The complete-JSON endpoint rejects continuation options.

When adjacent Responses text items are joined, their first item identity and
combined text are retained, and a supplied status from the latest fragment
replaces the earlier status. Completed continuation text therefore does not
retain the first fragment's incomplete label. Both applications share this
payload merge helper.

Upstream JSON remains buffered. Generated output is sent only after that
body is complete and passes the existing completeness checks, including tool
output. This adds downstream streaming without claiming token-level upstream
delivery. Continuation is explicit and retains the original account owner.

Earlier investigation notes are preserved in
[architecture history](history/architecture-before-reorganization-20261025.md).
