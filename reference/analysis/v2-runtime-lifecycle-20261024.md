# V2 worker cleanup and factory realm ownership

The shared goal is a fresh valid browserless mint and accepted native Framer
inference, with verified account isolation, matched identity/egress, cooldowns
and joined cleanup. Every lane contributes to that same outcome. A delivered
source change or passing synthetic operation does not complete the goal.
Accepted native inference remains unverified in the evidence recorded here.

The user's shared-goal policy prohibits voluntary blocked transitions because
a lane finished, a reply is pending, a cooldown applies, local work is exhausted
or another owner must act. Record the dependent action, its owner, required
input and last verified evidence. Continue concrete authorized work without
repeated status-only turns, session polling, duplicate runs or invented tasks.
Pause only on the user's request; complete only after verifying the full goal.

Preserve the existing goal identity, full objective and usage accounting. Do
not clear/recreate the goal or use `/goal edit` until an accounting-preserving
path is verified. This policy updates guidance, not the stored objective.
The structure lane owns prompt propagation; the transport lane owns T3
enforcement and AGENTS changes. Prompt text cannot override a higher-priority
native tool requirement: disclose any actual conflict and resolve enforcement
with the config owner. This note is not evidence that enforcement has changed.

Branch: `goal/browserless-v2-runtime`. Realm fixture source base: `1faffdd3`.
Source status: worker correction and three no-SDK realm cases owner-qualified.
Tests and SDK/runtime execution remain UNRUN here. Owner
`544711e3-0b76-4cb4-9732-124a0486847e` retains
qualification, integration, private state, accounts, services and requests.

The user reports R14 used dedicated configuration `8ab12cbb`: normal browser
and paired baseline returned 200 with output; isolated V2 completed and joined,
then native HTTP returned complete 403 Castle JSON. Cleanup was confirmed,
with no worker-stage failure. Group peak was 1,699,512,320 bytes, without swap
or limit events. That frozen run is unchanged. This correction does not explain
the 403 or establish upstream acceptance.

The original adapter removes a slot and increments `joined` in its `exit`
listener. Its separate `stop` promise awaits both `terminate()` and that exit.
`close()` previously awaited only stops for slots still in the live set.
Consequently, exit followed by close can omit an already-started stop whose
termination promise remains pending. A stop rejected after exit can also be
forgotten before close. These are static control-flow findings; no actual
Node-thread occurrence is claimed. The new inert gates model those orderings.

The correction retains outstanding stop promises separately from live slots.
Settlement removes each promise, and rejection retains one failure flag.
Close starts remaining live stops, awaits the retained set, restores the
original hooks, and refuses any retained failure. Failed termination still
awaits exit. No completed-promise history or raw error content is retained.

One-live-worker admission still uses the original live-slot set. Exit remains
the only source of a joined count; worker-error counts keep their original
meaning. Repeated terminate and close calls share their existing promises.
Hook restoration, source admission, message limits, resource quotas, native
Node/JSDOM identities, and operation deadlines are unchanged.

The pinned factory was read as source and its SHA-256 remains
`df5a150be914e7d267126f35de92934993e7930f18c3fd00bf79a86bd788ec58`.
The unchanged operation awaits `closeWorkers` before final device verification,
window closure and token release. Its two-turn drain is not a readiness proof.
Existing canvas, bounded PNG and real-worker qualifications are reused as
reported in development status. No qualification or SDK execution was repeated.

The first four inert cases cover pending termination after exit, failure before
close, failure during close, and failed termination still awaiting exit.
They also check restoration and truthful counters. All nineteen adapter cases
remain UNRUN here. Only source review, the factory hash and Git whitespace
checks were performed; no builds, tests, browser or runtime work ran here.

A fixture-only follow-up to `1a743756` adds two ownership cases. One admits
a replacement only after its predecessor exits, then requires close to retain
the predecessor's pending stop even after the replacement joins. The other
uses one adapter module with two windows: URL ownership, message delivery,
termination, restoration and cleanup failure must stay with their owner.
Both are UNRUN. They model local ownership boundaries, not verified account
isolation in a live service. Production source is unchanged by this follow-up.
The replacement case exercises the adapter's existing one-live-worker quota;
the dedicated mint profile still requires exactly one started and joined
worker before token release.

Owner-reported qualification: all 19 revised inert cases passed (`2cf7cae5`),
with a 47,435,776-byte peak, zero swap/limit events and absent unit afterward.
The prior two real-thread results are reused. None were run in this checkout.
The adapter entry remains:

```sh
node --experimental-vm-modules --test reference/fixtures/offline_v2_dedicated_workers_tests.mjs
```

Further qualification and package activation remain owner-held.

Further source inspection found a separate image-collector realm mismatch.
`v2_runtime.mjs` imports the factory as Node ESM, calls `factory(sdk)`, then
passes the JSDOM window to `configure`. That argument does not rebind the
factory's lexical globals. Both image round-trip collectors use the supplied
window for canvas creation but use bare `new Image` for image construction.
The factory contains exactly those two `Image` identifiers and no binding
for them. They resolve in the importing module's global environment.

In the same pinned factory, byte ranges are zero-based and end-exclusive:

- `r[gE]`: `[410593,411555)`, bare constructor at `411115`.
- `r[EE]`: `[411556,413706)`, bare constructor at `413101`.
- Registrations through `r[D5]` begin at `479932` and `480136`.
- `r[D5]`: `[394403,394689)` catches synchronous collector exceptions and
  invokes its completion callback with the undefined-result sentinel.

Both collectors test canvas/context availability first. If they reach a bare
constructor without a global `Image`, the synchronous failure precedes image
loading. It can therefore complete through the guarded fallback with zero
PNG requests. The earlier zero-attempt observations do not prove this path ran.

The pinned JSDOM `Window.js` defines `Image` at line 733 and installs it on
that window at line 752. Its SHA-256 is
`cfd16270beb33c10564b3474fd2e184b902f66f35eee2cab9191c699e36b4287`.
The V2 entry/runtime sources install no `Image` on Node's global object.
Passing or changing a function's receiver cannot change these lexical lookups.
The separate canvas-prototype probe at `408310` also uses bare
`HTMLCanvasElement` and `CanvasRenderingContext2D` inside catches.

This is source evidence of distinct lookup environments, not an observed
exception or a demonstrated cause of R14's refusal. Canvas and worker success
remain valid. No SDK, browser, test, private state or runtime was executed.

The new `reference/fixtures/offline_v2_factory_realm_tests.mjs` supplies that
inert discriminator using actual `createV2Window` and the owner's pinned
native-canvas JSDOM. This needs no SDK or inference call. A candidate correction would
evaluate the unchanged, verified factory module in the operation's own JSDOM
VM context, using `vm.SourceTextModule` and the genuine window objects.
It must be qualified separately before changing the selected profile.
Do not assign synthetic constructors or copy one account's globals into Node.
The factory, native identities, source hashes, quotas and deadlines remain
unchanged in this source contribution; runtime/operation edits stay owner-held.

The three cases evaluate the same tiny, import-free ESM factory first through
Node's data-URL import and then through `vm.SourceTextModule` with
`context: dom.getInternalVMContext()`. Expected facts distinguish bare lookup
from the supplied window's constructors, for both ordinary calls and explicit
window receivers. The two-window case checks that passing the other window or
using it as receiver does not transfer constructor or document ownership.
These are fixture expectations, not measured results. All three cases are
UNRUN here; source review and Git whitespace checks are the only local checks.

The factory returns fixed type strings and identity booleans only. Availability
checks retain native 2D canvas, absent WebGL and `CanvasRenderingContext2D`
global, absent workers/OffscreenCanvas/animation frames, and a hidden document.
The fixture assigns no constructors, reads or evaluates no SDK source, records
no pixel or exception values, and closes each created window in `finally`.
The separate runtime module loader and its fixtures remain the owner's work.

The owner qualified these three cases in its existing native-canvas pinned
package with the unchanged V2 constructor, using:

```sh
node --experimental-vm-modules --test reference/fixtures/offline_v2_factory_realm_tests.mjs
```

Owner receipt `2992def4` reports all three passing, peak 63,397,888 bytes,
zero swap/limit events and absent unit. Integration is `1a7d0df`. This confirms
bare `Image`/`HTMLCanvasElement` absence in Node and genuine own-window lookup;
global `CanvasRenderingContext2D` remains absent. Nothing was rerun here.

The owner's explicit `createRealmV2Minter` at `617ee44` then passed one
socket-denied synthetic SDK operation, receipt `db1eb359`: two PNG attempts,
two delivered and zero rejected, versus zero attempts previously. It reported
90 fields/567 bytes versus 88/563, with only additions 8/13 and 8/14, both
type 5 and length 2. One configure/mint took 799 ms in the method and 1,074 ms
overall, preserved the device, joined and discarded its token. Peak was
116,617,216 bytes with zero swap/limit events. Diagnostics were 1, async/late
were 0; the diagnostic count has no attribution. No live request was made.

A further static sequencing check resolves one possible callback concern.
The factory's `r[k6]` and `r[L6]` decode to `onload` and `src`, respectively
(assignments at bytes 148947 and 148995; string-table spans 16635:16651 and
16675:16683). In the two collector ranges recorded above, `r[gE]` installs
`onload` before assigning `src`; `r[EE]` assigns `src` first, then installs
`onload` before returning. Both construct their image with bare `new Image`.

For an admitted PNG, `resources.mjs` returns a native resolved Promise.
JSDOM `HTMLImageElement-impl.js:116` routes it through the per-document loader.
That loader queues it at line 77, and `resource-queue.js:115` invokes processing
from `request.then`. The per-document loader then dispatches `load` in another
Promise reaction at lines 55-57, after native image decoding. This chain cannot
deliver `load` synchronously inside the earlier `src` setter. The differing
assignment order therefore supplies no missed-callback defect on this path.
This is source reasoning, not a new callback observation or an acceptance claim.

Refusal is distinct: when the bounded loader returns `null`, the per-document
loader returns at lines 23-24 without queuing either event. Decode failure uses
its error path; these two collector bodies install no image `onerror` handler.
Neither possibility was observed here. No callback, timeout, quota or SDK
change is proposed. Window closure and process join retain their existing
owners; this source trace does not treat image delivery as proof of cleanup.

JSDOM source SHA-256 bindings for that trace:

- `HTMLImageElement-impl.js`: `e06dce97f0ee93c9ded67201a20f1069e2f38b2c023e2c77da1307782ccdcfe2`.
- `per-document-resource-loader.js`: `2437ba53ffecae4feb156890652dbc3d9f8697da1d522a579fd4b7325af083ac`.
- `resource-queue.js`: `526b1c0cb48b2bbab812f18580026e88314a41cb0b8dd9fb0f011a4bed0d7691`.

REA Evidence `ev_0932efa46003be8a7a01d77772520f7405e23f33c8f56af12dc24f163ae975c5`
parsed those three files as inert text, with zero parse failures. Semantic
coverage is partial and its unknown limit was reached; 493 other files were
outside selection. The ordering conclusion uses the exact source chain above.

Dependent action: runtime owner `544711e3-0b76-4cb4-9732-124a0486847e`
selects the next comparison. Required input is the owner-qualified realm
package plus retained account, identity, egress, cooldown and cleanup checks.
Last verified evidence is synthetic receipt `db1eb359`, joined with its token
discarded; the last live result recorded here is R14's native 403 refusal.
Future runtime source direction remains owner-held. This dependency neither
completes nor establishes a blocker for the shared goal. This note queues no
execution or inference request.
