# Serving architecture and branch policy

`main` is the browser-based service. `browser-based` preserves that path as a
maintained fallback. `browserless` is the separate development branch for the
native transport and short-lived SDK worker. A Git push does not deploy either
version or restart the running service.

The current parallel experiment keeps the live browser service unchanged.
A browserless instance needs its own port, database, profiles, sessions, gate
state and log directory, plus an account outside the live service's scope.
An instance selects a prepared host explicitly with `--sidecar-script`.
Generated hosts locate cooldown state from their canonical
`root/profiles/.sidecar/camoufox` path, rather than from shared source code.
They continue to reject stale or changed handoffs within their own instance.
An empty isolated instance can validate startup without claiming inference.
Session storage follows the selected `profiles/.sessions` directory. Native
admission also requires verified cooldown restoration for the host generation
and its actual selected device; account import alone does not establish these.
The process regression in `scripts/tests/run_parallel_startup.py` exercises
both the generated host and the built server in a private network namespace.
It places malformed handoff JSON beside shared code and verifies that the
instance ignores it, then checks input rejection with an empty account store.
Handoff JSON under the instance's own root remains subject to the existing
generation, inventory and cooldown checks. No stale record is deleted or
silently accepted as part of startup.

The separate restored-host process fixture exercises the generated package
with synthetic nonempty state. It verifies current device and rate ownership
through the real command pipe, including failed cleanup and stale/replayed
handoffs. It performs no SDK or upstream call and gives no live-account proof.
Final review accepts its source and six saved cases. The restored destination
is a Python fixture parent, and mint completion and cleanup replies are
injected. This does not validate a daemon recovery or real request cleanup.

The additional `run_restored_daemon.py` fixture starts the built server in a
private network namespace with complete synthetic account records. It binds
restoration to that server's actual process, exercises missing-session release
through normal streaming routes and preserves full-window and changed-JSON
refusals. It never supplies real session credentials or starts the SDK.
Final review `f299b5c3` accepts the fixture source and three distinct saved
cases. The original failed status assertion remains recorded; only the
corrected case was rerun. This validates synthetic restoration and device
release through the server, with no authenticated delivery or deployment claim.

Browser dependencies and optional browser memory patches are prepared only
when `_launch` first needs them. Native device and restoration commands do
not import Camoufox or Playwright. The browser path retains its transport
verification, per-account launch options and close ownership. Missing browser
dependencies refuse a browser launch without preventing native host startup.
This host change is validated in isolation and has not been installed.
Final review `35d0b188` accepts its source and saved evidence. The existing
server binary still embeds the old default host, so this candidate requires
explicit `--sidecar-script` selection until a later executable rebuild.

Normal API routing additionally requires `--offline-routing-config`.
`examples/browserless-routing.json` provides bounded, unpaid streaming limits;
it is not installed automatically. Native requests also require explicit
`x-framerize-context` metadata for their page and attempts. The separate
`run_native_binding.py` fixture verifies that a normal streaming request reaches
native dispatch with synthetic context and an empty account store. It proves
the routing binding, not authentication, upstream acceptance or text delivery.
The same configuration was applied only to the isolated port 8788 instance
at 2026-10-23 16:07 UTC. That instance had a 30-minute lifetime; this saved
observation does not establish current availability.
See [parallel validation](parallel-browserless-validation.md) for the remaining
account, streaming, cleanup and resource evidence required for a real run.

The native stream ownership observer records `native.stream.complete` only
after both body completion and independent cleanup join. Body failure and
cleanup confirmation remain separate fields. Only bounded scalar data is
selected; provider IDs, headers, usage and content are excluded. The observed
attempt count covers retained metadata, and yielded bytes are not proof of
delivery. An absent record does not prove that no attempt occurred.

Native HTTP also increments a process-wide counter immediately before each
send invocation, including attempts that fail before response headers. It emits
`native.http.send` with that counter. The public raw and composed stream
adapters emit `native.stream.sends` after the original cleanup resolves, with
the cumulative count and unchanged cleanup outcome. Internal composed steps
do not create additional public cleanup snapshots. These events contain no
account, route, credentials or provider data. They do not prove upstream receipt.
The isolated checker requires sequential counts one and two and matching
cleanup snapshots on a fresh, exclusively owned generation. Missing or extra
evidence refuses acceptance; the existing body and pool checks remain required.

The optional `scripts/check_parallel_native.py` client binds an explicitly
separate service generation on loopback port 8788. It operates no services
and never contacts the browser endpoint. A generation marker prevents
replaying the same check under a different report name. Its two requests
share one account: interrupted streaming first, then complete streaming only
after joined cleanup is observed. A retained socket permits interruption
after response headers detach the original connection. Quiescence requires
explicit idle status with no pending replacement as well as zero users and
no cleanup uncertainty. An empty pool is also allowed. Reports contain
bounded counts and local error codes, without provider content. Failed
sequences only observe existing cleanup and do not retry inference.

The checker starts each request's monotonic clock before sending its HTTP
request. Stream snapshots record elapsed time to headers, first complete text,
terminal completion, socket disconnection and reader exit. Heartbeats and
partial text frames do not establish first text. These are client observation
times, including parsing and scheduling; reader exit does not prove server
cleanup. Existing native completion and idle-pool checks remain authoritative.
Final review `d9760e73` accepts the timing source and saved synthetic results.
The prepared client is pinned separately from the unchanged staged executable;
neither that preparation nor synthetic timing validates a live generation.

The separate offline worker probe measures a local SDK invocation with
synthetic input and no account or network access. It retains the worker's
syscall filter and records process RSS separately from group memory. Its
heap-policy experiments do not change the production launch configuration.
See the [worker memory results](offline-worker-memory.md) for the measured
limits and rejected settings.

The original six-run worker study has accepted source and saved evidence.
Later probes attribute an early allocation increase to importing JSDOM.
A read-only static Node cache slightly reduced local minting time across
three comparisons but increased group memory. Those results are locally
verified and not independently reviewed. Production retains the 96 MiB heap
and does not enable that cache. An additional verification-buffer experiment
passed its local mint but demonstrated no useful saving and remains unapplied.
The import-only probe records bounded package and module allocation observations.
It invokes no SDK and restores its loader hook. Its largest individual net
heap increase coincides with CSS style prototype setup. GC and nested loading
prevent treating these observations as exact ownership or achievable savings.
An isolated CSS prototype experiment preserved the tested descriptors and
setter observations, but its import-stage reduction did not yield a useful
full-mint saving. It remains an unapplied reproduction patch. These direct
probes do not measure the Rust owner or the private pipe integration.
The measurement tool also permits a JITless comparison. Its first synthetic
sample used less memory and took longer than the saved baseline mean.
That flag is not selected by the production worker; see the measurement
report for the separate process/group results and their limits.
Two subsequent local samples retained the memory reduction. The three-sample
mean group peak is 57,839,616 bytes, 22.90% below the reused baseline mean,
with 3.65% more local minting time. The actual owner/host/request integration
has not been measured with this flag; production still uses its original policy.

The separate `measure_owned_offline_worker.rs` harness links the unchanged
owner library from the accepted application build. It runs the original
worker entry with synthetic device/project input, exercises the real private
pipe and metadata/freshness checks, waits for joined completion, then clears
and discards the token. One isolated run passed with a 75,116,544-byte group
peak and no remaining worker, unit or cgroup. The harness exercises no account
or host admission, HTTP request or application service. Final review
`089c5975` accepts its source and saved build/run bindings. It does not change the
96 MiB production heap or establish total service memory or request latency.

The companion `measure_owned_offline_cancel.rs` exercises explicit cancellation
and owner drop against the real worker. It requires observed child activity
and a stable start identity, then verifies joined cleanup and child absence.
Both local cases pass with the original worker and unchanged owner library.
They identify no SDK phase and do not replace a real HTTP stream-disconnect
check or establish later account admission. Final review `72ba6876` accepts
the source and saved evidence without findings.

The separate `run_host_worker.py` fixture combines an accepted native host
with that same owner executable. It restores synthetic state, holds a device
lease across one real local mint and reports joined completion before release.
It then opens and closes a fresh lease without another mint. Host and worker
have separate filesystem views, and neither can reach real account state.
One combined group measurement passed at 102,887,424 bytes peak, including
the measurement helpers. It excludes the application server and authenticated
inference. Final review `d98b96cf` accepts summary `6af8688a` and the
frozen source and saved evidence, with no findings.

The production `OfflineWorkerMinter::with_device` bridge waits for the
worker to join before reporting completion to the shared host, then retains
account preparation until the host acknowledges it. New focused tests cover
explicit cancellation, a dropped result waiter and a lost host acknowledgement.
They use the real worker owner and account preparation with a deterministic
worker and controlled host replies. The source is prepared but UNRUN:
three compilation admissions were refused before launch by the memory reserve.
The corrected validation launcher uses fresh labels under the original output
parent, with three labels per mode and all resource guards preserved. Its result
binder also checks the exact launcher, dependencies, label and mode. This
additive preparation correction is accepted for source/static bindings and
remains unexecuted because the compiler reserve has not been met.
These cases do not establish authenticated delivery or actual host cleanup.

```mermaid
flowchart LR
    Client --> Server[Shared HTTP API and account selection]
    Server --> Pool[Account, capacity and cooldown ownership]
    Pool --> Browser[Browser path on main and browser-based]
    Browser --> Upstream[Framer]
    Pool -. experimental branch .-> Native[Native transport and SDK worker]
    Native -. same request .-> Upstream
```

The browser path uses each account's own session, proxy and browser state.
The resident pool limits how many browsers stay loaded. Request ownership
continues through response streaming and cleanup; uncertain cleanup keeps
the account fenced instead of making it immediately available again.

The static hello responses carry explicit completion evidence because the
router strips HEAD bodies after request ownership has wrapped them. Initial
wave and account-selection refusals attest completion only at their terminal
return, before residency or external work. The middleware preserves stronger
response evidence. Body size, a HEAD method, or a generic error does not by
itself establish that work has finished.

The browserless path reuses the existing admission and session boundaries.
It must retain the account's device and cooldown authority. Parallel instances
use disjoint account ownership; an independent copy of a live account's state
does not establish that ownership. A bounded real test sends one request through one account owner,
without duplicating paid traffic.

Application code stays in the existing crates: core owns domain policy, server
owns HTTP routing, API owns Framer protocol details, host owns browser/device
integration, and the executable assembles them. Add missing connections at
these boundaries instead of introducing another ownership framework.

## Experimental device lifetime

The native request obtains its saved device from the same restored host that
owns browser cooldowns. The host admits and charges each mint before the SDK
worker starts. It refuses browser work for that account while the native
request remains owned, so the two paths cannot spend independent rate limits.

The R18 source carries that same saved device from the joined worker
result into the private request cookies. It preserves the original login,
rejects conflicting or duplicate device cookies, and never writes the request
snapshot back to the session store. The existing account-generation and private
session checks still apply when those cookies reach the HTTP transport.
This connection passed the selected local cookie and authorization tests.
R18 deployment is verified. Its first trial refused local admission; upstream
acceptance remains unverified.

The application keeps the host grant until the worker, native session and
response cleanup have finished. Cancelling the client does not release that
grant early. A lost cleanup acknowledgement leaves the account fenced.
Successful stream headers can still reach the client before the body ends.

The deployed disconnect correction separates cancelling future work from
confirming current work. Browser chains stop before another attempt, but keep
the already-started call owned until it returns. Native responses use their
existing completion and cleanup observers. Dropping a response does not itself
prove cleanup, and it must not start another mint or drain an abandoned chain.

For cancellable browser calls, the host returns after one attempt. Rust checks
whether the client is still present before authorizing a retry or optional
refund. An already-started call retains its owner through the real reply, with
one overall deadline. An unknown timeout or refund cleanup remains unconfirmed.
Legacy raw and paid response paths without observers retain their prior limits.

Buffered normal responses carry the observed completion of every account
attempt, including an earlier attempt whose cleanup remains uncertain. A
later successful answer cannot clear that uncertainty. Client disconnect
stops future attempts while the current browser call keeps its owner.
Retry-admission and serialization failures retain that same accumulated
completion outcome, including when the error response is fully consumed.
The browser host records a job-start event after the page has started its
fetch. This lets a live cancellation check observe the actual call boundary
without logging request content, credentials or a response body.

The normal-route recovery check disconnects one unmetered request after that
start event, waits for its actual completion and cleanup, then requires a
complete second response on the same process and runtime configuration.
A restart between those requests cannot establish cancellation recovery.

The streaming acceptance client retains its own socket descriptor after HTTP
headers arrive, so closing the connection still interrupts the live response.
Opening frames do not count as completion. A successful stream needs its
terminal completion event, nonempty text and a clean HTTP ending. The first
stream includes the hello HEAD probe after the observed browser fetch start;
the second stream follows observed call completion and local cleanup.

An admission-only executable replacement keeps the current browser host,
support files, arguments and service overrides. The deployment controller
may adopt an abandoned maintenance marker only after checking its exact
identity and absent prior owner, while holding an exclusive file lock.
Startup, cooldown restoration, unchanged account and configuration state,
idle residency and zero current or historical swap are required before release.
The bounded startup receipt contains a count and hash of its source map;
the controller still hashes every declared source before accepting a phase.
The normal-stream client verifies that release receipt, accepted plan and
current generation before sending a request. Its owner runs deployment first
and permits one client invocation, retaining a consumed marker on refusal.

After a completed rollback, a separate helper can verify the restored
generation and release the retained hold without starting or stopping the
service. It verifies saved startup coverage against the same log bytes,
cooldown restoration, pool residency, protected files and zero swap. It
repeats current generation, protection and drain checks before both handoff
clearance and marker removal. The runtime owner joins peer silence while
these checks run; a maintenance marker alone does not stop inference traffic.

Worker protocol version 2 carries the private saved device through a pipe.
The worker seeds its isolated storage before loading the SDK and checks that
the value remains unchanged. This does not establish upstream acceptance or
prove which identity an opaque token contains. Those require separate evidence.

The worker's JSDOM window identifies the application as `Framerize/0.1`, the
same identity used by both native HTTP clients. Its resource loader always
returns no resource, including for HTTPS, file and data URLs. This preserves
the isolated page context and does not make JSDOM a browser or prove that
an upstream security check will accept a native request.

The generated host package installs the native device adapter after the
existing restoration hook. A base embedded host alone does not provide this
integration; native configuration remains an explicit deployment choice.

The first trial enables `/api/offline/complete` in that same daemon. Ordinary
inference routes keep the browser provider. The trial receives explicit
request context and returns a completed response only after native cleanup.
Enabling its worker configuration does not switch the normal routing policy.
This lets one real request exercise the shared ownership without running a
second account authority or sending duplicate traffic.

After that request and its cleanup settle, `offline.complete` records only
the upstream status, attempt counters, refusal classification and final cleanup
outcome. It excludes response text, model strings, account identities and request
context. The final cleanup outcome can remain false even when an inner attempt
reports successful cleanup. The event is diagnostic evidence, not a retry signal.
An upstream security refusal and successful cleanup are separate facts. The
refusal classification does not identify whether device state, request context
or another upstream check caused it. Keep that distinction when selecting the
next source correction; a completed rejected attempt is not delivered output.

A browser and native grant cannot use the same account simultaneously. After
the existing host confirms browser closure, the native adapter retains that
browser's actual final device selection in the shared lifecycle. Cooldowns and
request history remain in that same lifecycle. Failed closure transfers no
authority, and an account without a recorded primary device remains ineligible.
The first trial needs an idle account parked through the normal pool owner;
general account availability alone does not establish native readiness.

The deployment capture must preserve a parked account's recorded primary
device from its lifecycle. A resident browser's actual selection takes
precedence. The additive capture correction covers both cases and preserves
null or missing selections as unknown. It retains every cooldown and request
timestamp. This prevents another state transfer from discarding a known
selection; it does not reconstruct one already lost or make an unknown device
eligible. The corrected extractor has passed its 17 focused fixture cases.

An already-lost selection must be established by the normal account owner.
The separate preparation client uses one bounded free browser request before
parking that same account. It checks the service generation around both
operations and retains the existing native client and its release guards.
Its event evidence is bounded and excludes response content and credentials.
Incomplete or ambiguous evidence prevents the subsequent native request.
The preparation client's fixture pass does not establish live readiness.

The client-only attempt also checks the current released generation, protected
runtime files and resource limits before sending either request. A refused
attempt remains consumed. Service swap requires separate recovery evidence;
closing browser residents alone cannot clear swapped server or sidecar pages.
After any restart, the client must bind the newly verified generation before
preparation and delivery, while the coordinated quiet window remains in place.
The original release remains provenance. The recovered generation must come
from the accepted recovery plan and its completed controller, state and release
records, then match the live process pair and unchanged support/configuration.
The original release's old process identity cannot stand in for that proof.
The saved inspection and release records wrap their protected fingerprint
together with the release pin. The client compares those complete wrappers,
then binds the fingerprint and release to the recovered state. The live
configuration check continues to compare the bare fingerprint.

The local owner sequence retains its authenticated terminal session while
placing each child phase in a separate process group. Cancellation signals
that group and waits for the child. Starting a new session discards the
terminal authentication context even when the caller has authenticated.
An authentication refusal consumes that attempt before recovery or requests;
preserve its evidence and bind any later attempt separately.

The recovery identity helper uses the recovery controller's own binding
module. Loading a deployment helper in a different controller does not provide
its original import namespace. The recovery package verifies every selected
module through the real loader without live actions, and retains the original
helper's import failure as a regression. Those import checks do not exercise
identity checks against installed state or replace fresh runtime predicates.
The client and owner must bind the same accepted recovery directory and plan.
Updating those references preserves prior consumed attempts and carries the
new helper hash through the client and input driver before execution.

Recovery of an already installed runtime verifies its configuration in place.
It preserves both file content and identity, obtains a fresh maintenance hold,
and reuses the drain, capture, settlement and startup ownership checks.
Its input verification follows the previously accepted handoff archive after
clearance. Releasing a new hold still requires confirmed startup and cleanup.
Launch capture for recovery binds its current executable explicitly to the
installed release's verified binary. The deployment adapter's older executable
constant remains a rollback binding. Passing a replacement hash alone does
not change the executable that capture expects to be running. The recovery
copy retains the original pool, process-generation and startup checks.
Source-only acceptance cannot open the recovery execution gate. Its final
plan must bind the completed capture fixtures and retain the original saved
recovery cases. Execution still requires a separately accepted plan, fresh
resource and generation checks, and actual drain under the existing owner.

A support-only release can reuse the verified Rust binary. Bind its changed
support file to separate review evidence, and regenerate only the host's
support-manifest reference. Preserve the previous launch override for rollback.
Carry forward the complete set of retained configuration and override files
from earlier releases. Check every recorded hash and the exact directory
contents before changing the service; keeping only the latest pair loses
the protection for older files that are still installed.

Two releases may use the same executable with different support packages.
Rollback therefore identifies the running generation and launch configuration;
an executable hash alone cannot establish that the old release is running.
The current rollback restores the original executable and launch configuration
while retaining the new restoring host and support package. It preserves the
existing cooldown transfer and verifies the resulting runtime explicitly.

Deployment also has to account for coding sessions using the same daemon.
A maintenance marker controls recovery ownership; it does not stop incoming
requests. Permission to cancel a request is not proof that it has finished.
The transition must still observe fresh zero active work and zero ingress
before capturing state. Preserve a refused execution and arrange a separate
coordinated attempt instead of weakening those checks or overwriting evidence.

Quiet must last through startup, including work generated by the runtime
owner's own coding session. A new service generation can accept requests while
its browsers are still warming. Saved request events must distinguish that
activity from an actually quiet startup; an acknowledgement is insufficient.

A failed startup can leave a restored, serving generation and an owned
maintenance hold. Recovery verification must bind the exact current generation,
original launch configuration, account isolation and restored cooldown state.
Releasing verified recovery bookkeeping does not turn the failed startup into
an accepted deployment or establish browserless delivery. Preserve both results.

Swap in a restored generation can prevent its hold from being released.
A separately bound recovery may treat that exact swapped generation as its
input, then reuse capture, settlement and startup for the installed runtime.
It must preserve cooldowns, account identity, runtime files and hold ownership.
Successful warmup, idle residency and zero current and peak swap are still
required afterward. Accepting a degraded input is not acceptance of a degraded
result. Save new recovery evidence separately from the failed deployment.

Account selection must also remain stable over the accepted startup interval.
A quiet client window does not clear cooldowns or replace that requirement.
Use the recorded expiry times to choose the next attempt, then let the
launcher check current state again. Keep original cooldown timestamps and
the same shared account authority throughout the transition.

Completion evidence belongs to one exact service generation. A later recovery
or restart invalidates its use as current release evidence even when the binary
and sidecar hashes are unchanged. Check the current process, installed files,
accounts, pool, restored state and maintenance ownership before a live test.
Never recreate an old pending handoff merely to make a verifier pass.

A deployment prepared after recovery also binds its preparation and baseline
to the recovered generation. Identical payload bytes do not make the earlier
process identity current. Preserve the old failure and recovery records in the
new release's evidence, and repeat the normal readiness checks before launch.

A refusal before controller launch does not consume a deployment execution.
Preserve that refusal and any expired coordination window. If the missing
resource is restored without changing accepted inputs, record a separate
attempt with fresh coordination and admission against the same accepted plan.
The disk reserve stays fixed; removing verified disposable cache does not
authorize changing it or deleting runtime state and frozen evidence.

The runtime support manifest includes six files, including `native_device.py`.
The completion checker requires all six and verifies their hashes, ownership,
permissions and exact directory membership. Archived handoff copies may prove
historical execution only. Such a result must carry a distinct status that the
live release command rejects.

The optional request observer wraps the existing client checker, verified by
its exact source hash. It samples the shared service's memory during that same
request and retains measurements on failure. It neither owns accounts nor
changes request policy. Its sampled maximum is a lower bound, and concurrent
browser work prevents attributing that maximum to the native request alone.
See [browserless-measurement.md](browserless-measurement.md) for interpretation.

## Moving the default

Keep normal browser serving available throughout development. Before moving
`main` to browserless, demonstrate real client delivery, streaming, tool
continuation, cancellation, cleanup and account isolation. Measure the whole
service, including its worker and host processes. An isolated worker peak is
not the service's total memory use.

The wider goal still includes 100 MB total memory, four simultaneous delivered
streams, a 30-minute soak and measurements across ten accounts. These remain
targets, not established results. Preserve the last working browser commit on
`browser-based` when making a later reviewed default change.

## Continuing work around a waiting step

Project progress and runtime admission are separate decisions. A deployment
can wait for authentication or stable account selection while other useful
engineering continues. Keep that dependency attached to the deployment;
never voluntarily treat it as a reason to stop the entire project.

Use the current source and saved evidence to select the smallest independent
implementation or measurement that advances the original objective. Keep an
explicit owner, expected result and next action. Reuse valid results instead
of repeating reviews or polling unchanged prerequisites. A resumed request
requires a fresh assessment of available work.

This does not authorize a second account owner, duplicate traffic, weaker
admission or invented results. Follow mandatory goal-tool status rules only
when their full conditions apply, and explain any required transition.

## Publishing work

Commit and push each coherent milestone to its appropriate branch. Include
the checks performed, any reused evidence and the remaining limitations.
Update `development-status.md` before a handoff. Preserve the original frozen
evidence locally; publish source and a concise account of results, not private
runtime captures or account data.

The browser source includes the base Python host embedded by Rust. The deployed
service also uses a generated cooldown-restoration hook and five support
modules. Keep that runtime packaging distinction explicit when building or
releasing; copying the embedded base host alone does not reproduce the entire
deployed package.
Use `scripts/prepare_browser_runtime.py` to prepare those tracked sources in
a new directory. See `scripts/cooldown_support/README.md` for the boundary
between package generation and an actual coordinated deployment.
