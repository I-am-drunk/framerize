# Realm status and the actual system

Where the copy approach stands. The no-JSDOM realm answers every SDK
question with the user's real Chrome values and replays the captured 324-
read canvas probe table. It is complete except for one defect: under full
six-module graph evaluation in `node:vm`, the bundled string-table decoder
throws `ReferenceError: atob is not defined`, even though `atob` resolves
as a function in isolated probes of the exact same context (single module,
`runInContext`, and per-module evaluation all pass). This is a Node vm
module realm-record interaction, not a missing value. It is fixable but
expensive to chase, and patching around it (transforming the bundled
decoder) drifts from the pinned bytes.

Why the actual system is the better build. The copy path mints once and
replays. The actual system is a small persistent Node process that owns a
no-JSDOM realm, mints a fresh token on each request over a loopback socket,
and hands it to Framerize. RAM stays near 100 MB because there is no
browser and no per-request JSDOM. The replay table and real-browser values
are captured once at setup (and refreshed if Chrome updates), not on every
request.

The service reuses everything already proven: the module-chain runner, the
collection handoff, the captured `real-browser-values.json` and
`probe-capture.json`, and the realm globals in `realm.mjs`. The only open
item is the vm `atob` defect, which the service hits the same way unless it
evaluates the graph as a script (no ES modules) or the defect is resolved.

Immediate next move: try script evaluation of the graph inside the realm,
which sidesteps the vm module realm record entirely. If the graph runs as
a classic script, the service works today.
