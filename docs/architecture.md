# Architecture

The current reorganization is in progress. See [the design](rearchitecture.md)
for decisions, boundaries and the remaining migration work.

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
it never loads the SDK module. The serving worker remains unchanged until the
complete native replacement is implemented and validated.

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
service acceptance. The native serving worker does not use it yet.

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

Each native service request creates an internal random diagnostic ID. Its
credential source passes that ID to the worker minter, and JSON or SSE
completion logging uses the same ID. These IDs contain no account or inbound
request identity. Worker failures retain only validated phase facts: fixed
labels, booleans, bounded counts and finite timings. Pipe-writer entry and
return are distinct from age-qualified handoff and parent process cleanup.
This correlation does not add observations of unreported SDK or server phases.

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
