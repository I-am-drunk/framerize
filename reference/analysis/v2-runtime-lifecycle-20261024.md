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

Latest published live evidence is R15, receipt `8580c020`, recorded in
`d389216`: normal and baseline browser requests returned 200; the explicit
window-realm native request returned complete Castle 403 JSON. Cleanup was
confirmed, the isolated host stayed fenced and the protected service was
unchanged. Binary `345cf340`, realm package `514c442b` and all 1,441 activation
inputs matched. Peak group memory was 1,584,971,776 bytes, with zero swap or
limit events. No repeat request is queued.

R15's browser baseline has 105 fields/817 bytes; its actual native token was
not projected. The earlier synthetic realm receipt `db1eb359` describes a
different token and cannot supply that missing native observation. Neither
matching capabilities nor the available structural comparison establishes
why R15 was refused. The owner has assigned the bounded exact-token projection
port to the structure lane and retains caller integration and qualification.

Earlier R14 used dedicated configuration `8ab12cbb`: normal browser
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
They also check restoration and truthful counters. The owner qualified all
nineteen adapter cases in receipt `2cf7cae5`; they were not rerun here.

A fixture-only follow-up to `1a743756` adds two ownership cases. One admits
a replacement only after its predecessor exits, then requires close to retain
the predecessor's pending stop even after the replacement joins. The other
uses one adapter module with two windows: URL ownership, message delivery,
termination, restoration and cleanup failure must stay with their owner.
Both passed in that owner qualification. They model local ownership boundaries,
not verified account isolation in a live service. This follow-up changes no production source.
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

Source inspection found an image-collector realm mismatch in the default Node
profile: it imports the factory as Node ESM, calls `factory(sdk)`, then passes
the JSDOM window to `configure`. That argument does not rebind the
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
remain valid. No SDK, browser, test or runtime was executed in this checkout.

The new `reference/fixtures/offline_v2_factory_realm_tests.mjs` supplies that
inert discriminator using actual `createV2Window` and the owner's pinned
native-canvas JSDOM. This needs no SDK or inference call. The owner's separate
`createRealmV2Minter` at `617ee44` evaluates the unchanged, verified factory in
the operation's own JSDOM VM context through `vm.SourceTextModule`. Its
synthetic qualification is recorded below; the default Node profile remains.
Neither path assigns synthetic constructors or copies account globals into Node.
The factory, native identities, source hashes, quotas and deadlines remain
unchanged in this source contribution; runtime/operation edits stay owner-held.

The three cases evaluate the same tiny, import-free ESM factory first through
Node's data-URL import and then through `vm.SourceTextModule` with
`context: dom.getInternalVMContext()`. Expected facts distinguish bare lookup
from the supplied window's constructors, for both ordinary calls and explicit
window receivers. The two-window case checks that passing the other window or
using it as receiver does not transfer constructor or document ownership.
The owner verified these assertions in receipt `2992def4`. Local work remained
source review and Git whitespace checks; no qualification was repeated here.

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

The realm profile also changes timer ownership. All 12 `setTimeout` and 12
`clearTimeout` identifiers in the pinned factory are bare calls, with no local
binding. Examples are the deferred helper at byte 158300, timeout callbacks
at 393735 and 431691/432451, and deferred calls at
537730/538451. These resolve in Node for the default profile and in the owned
window for the explicit realm profile. Passing `window` to `configure` alone
does not change the default profile's timer ownership.

The same pinned JSDOM `Window.js` owns a timer map at line 521, registers each
Node timer at lines 600-601, and skips callbacks removed from that map at line
573. `window.close()` calls `stopAllTimers()` at line 878; that function clears
every registered Node timer and the map at lines 682-686. A pending bare Node
timer from the default factory is outside this window map. A bare timer from
the own-window factory uses it, without replacing Node globals or changing
the SDK's requested delays. This is source-derived lifecycle behavior.

No pending timer, missed deadline or unjoined process was observed by this
trace. The existing outer deadline, cancellation and process join remain
required for both profiles; window closure alone is not a child-join receipt.
No timer shim, delay adjustment, extra execution or new acceptance claim is
introduced. This also supplies no attribution for the diagnostic count or 403.

Dependent action: runtime owner `544711e3-0b76-4cb4-9732-124a0486847e`
integrates and qualifies the planned projection at the actual native caller.
Required input is the structure lane's bounded decoder and the existing token
and lease, with all account, identity, egress, cooldown and cleanup checks kept.
Last verified live evidence is R15 receipt `8580c020`: native 403 and confirmed
cleanup, with no actual native-token projection. `db1eb359` remains synthetic.
Future runtime source direction remains owner-held. This dependency neither
completes nor establishes a blocker for the shared goal. This note queues no
execution or inference request.

The worker-reply realm boundary is now traced at message ingress. In the pinned
factory, `t[s4]=X(i(32063,34815))` at source bytes 396620 onward decodes as
inert text to the admitted 1,031-character dedicated-worker script. Its SHA-256
is `11bba9bb64764c19f44154854b4ec9f66756a6ef95de25ef1a804db38c30b9bd`,
the exact script admitted by the existing runtime. It constructs arrays for
both normal replies and its caught-error reply. The first normal array includes
navigator values and a languages array; the second carries high-entropy results.
This describes source structure, not observed values or successful execution.

The adapter passes the Node worker's message directly as `MessageEvent.data`.
JSDOM's `MessageEventInit.js:15` uses Web IDL `any`, whose conversion returns
the original value (`webidl-conversions/lib/index.js:166`). `Event-impl.js:16`
stores that value, and `MessageEvent.js:248` returns it. This path performs no
conversion to a window-realm Array. Existing window event ownership is retained.

The dedicated reply handler `r[Ib]`, bytes `[431159,431896)`, first reads
`Array[X(p9)]`. Static string decoding resolves `X(p9)` to `isArray`
(`p9` assignment at 118371, pool `[3295,3315)`). `Array.isArray` accepts arrays
from another realm; this check does not require window Array prototype identity.
The fallback tag decodes to `[object Array]` (`A9` at 118387, pool `[3315,3355)`).
Thus the retained Node array is not refused solely for its realm at this ingress.
This does not establish downstream value parity or explain a Castle refusal.
No payload conversion, constructor copying or runtime patch is justified here.

The factory's existing REA record is
`ev_db2b0896c9c56156aa251b2a3f14b9b2e2a482291a67382cb72608aff6d4b663`.
Its source analysis timed out with incomplete coverage and was not repeated.
This finding uses direct pinned-source reads and static string-data decoding;
no SDK code was evaluated. All test and runtime execution remains UNRUN here.

Dependency SHA-256 bindings for the message-ingress trace:

- `jsdom/lib/jsdom/living/generated/MessageEventInit.js`: `f556bd31ab9047c186a0002feeadf35a5f8374ad8f1272ab6b082b5a21c6a270`.
- `jsdom/lib/jsdom/living/generated/MessageEvent.js`: `fc1e07e1ce960a01dac6655b20b407e6209586c4cd9353b0abe5f112b4f309c7`.
- `jsdom/lib/jsdom/living/events/Event-impl.js`: `5b0225f05d06a200aaa8e0760cfcb4df3ccb04d73c2bacc951a053f33e7aa90e`.
- `webidl-conversions/lib/index.js`: `c3b203d992b46905b610d240313d72e96d04b756942d9612878104ee12b107d4`.
