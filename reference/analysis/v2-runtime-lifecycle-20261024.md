# V2 dedicated-worker cleanup ownership

Branch: `goal/browserless-v2-runtime`. Source base: `71b6326`.
Status: source correction and six inert regression cases prepared; all cases
UNRUN here. Runtime owner `544711e3-0b76-4cb4-9732-124a0486847e` retains
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

Pending qualification: the runtime owner runs the revised inert adapter suite
using its existing owned test environment:

```sh
node --experimental-vm-modules --test reference/fixtures/offline_v2_dedicated_workers_tests.mjs
```

Any actual-thread qualification or package activation remains owner-held.

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

Exact next action: the runtime owner uses an inert factory-shaped module to
compare bare `Image` lookup with the supplied `window.Image` under the existing
loader. This needs no SDK or inference call. A candidate correction would
evaluate the unchanged, verified factory module in the operation's own JSDOM
VM context, using `vm.SourceTextModule` and the genuine window objects.
It must be qualified separately before changing the selected profile.
Do not assign synthetic constructors or copy one account's globals into Node.
The factory, native identities, source hashes, quotas and deadlines remain
unchanged in this source contribution; runtime/operation edits stay owner-held.
