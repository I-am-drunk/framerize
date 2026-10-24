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

Exact next action: the runtime owner qualifies the revised inert adapter suite
using its existing owned test environment:

```sh
node --experimental-vm-modules --test reference/fixtures/offline_v2_dedicated_workers_tests.mjs
```

Any actual-thread qualification or package activation remains owner-held.
