The standalone V3 service packages this worker with Node and its dependencies.
Build it with Node 24.13.0:

```sh
node scripts/offline_worker/bundle.mjs --output target/native-worker
python3 scripts/package.py framerize-native
```

The builder verifies pinned public assets and uses `npm ci --ignore-scripts`.
It never opens an account or runs the SDK. Package configuration resolves
paths from its own directory; the build machine's paths are not retained.
See [application packages](../../docs/releases.md) for offline cache options.
The complete bundle contains the V3 entry point. The V2 source and graphics
experiments below remain separate and are not enabled by this package.

Build the owned SDK probe with
`cargo build --locked -p framer-native --example worker-smoke`. It uses the
service's configuration parser and expects a read-only worker bundle mounted
at `/work`, plus a fresh writable `/out`. It refuses to run unless network
and PID namespaces differ from `FRAMERIZE_PARENT_NETNS` and
`FRAMERIZE_PARENT_PIDNS`, private home state and the user bus are hidden, and
its cgroup has 128 MiB memory, zero swap and 32 tasks. The runner also verifies
no-new-privileges before execution. Its JSON result contains only timing,
failure and cleanup facts. The synthetic token is discarded after handoff.
This probe has passed with the portable V3 bundle; it does not contact Framer.

This source adds explicit saved-device storage to the isolated worker.

The SharedWorker adapter executes the pinned SDK worker script in a real Node
worker thread with native message ports. Only bounded, locally registered Blob
URLs are admitted. Each window allows one worker and four connections; separate
windows do not share state. The worker reports the truthful Framerize/0.1 user
agent and leaves userAgentData absent. It cannot load a network script.
Mint completion requires the expected reply and joined thread cleanup. The
128 MiB group bound remains authoritative; Node command-line heap settings can
override the worker's requested per-isolate heap limit.

The browserless worker now permits bounded inline PNGs for a separately pinned
native canvas bundle. External URLs, local files and other data formats remain
refused. Each window has its own four-image quota; each PNG is limited to
65536 bytes, 2048 pixels per side and 262144 pixels total. Unsupported drawing
contexts still return null. File verification uses a reusable 64 KiB buffer.

The deployable canvas bundle passed all 34 component cases and one synthetic
mint through the existing JITless owner, including private token handoff and
joined cleanup. The mint peaked at 84516864 bytes, with zero swap or memory
events and an absent unit/group afterward. Tests requiring drawing use the
qualified JSDOM 26.1.0/node-canvas 3.2.3 bundle and its context patch. The normal
pins remain unchanged until an explicit bundle is selected. These results do
not establish Framer acceptance or change either running service.

Version 2 requires one supplied device value. The worker seeds localStorage
and the __cuid cookie before evaluating the original SDK wrapper, then checks
both values before closing the window. Missing, malformed or changed values
fail the operation. The device handle rejects JSON serialization, and public
metadata records only seeded/unchanged booleans. Version 1 retains its original
stateless behavior and account_state_loaded remains false in both versions.

The SDK, wrapper, branch observation, token writer and process cleanup stay
unchanged. readInput now returns parsed input for the constructor's single
validation, preserving the private handle through its lifetime.

The 19 existing inert cases remain. Nine additional cases cover explicit input,
privacy, real JSDOM storage, separate windows, repeated saved values and rejected
storage changes. No case evaluates the SDK or contacts a service.

The 28 inert component cases passed on 2026-10-22. This component accepts a
value; the application obtains it from the selected account's existing host
authority and rate ledger. Combined application validation remains separate.
It does not establish token-internal identity, upstream acceptance, account
continuity, live integration or deployment. All earlier freezes are preserved.
