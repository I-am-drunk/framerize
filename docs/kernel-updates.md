# Kernel and application updates

Worker Data uses the same two downstream slots as legacy0. A synchronous
reader dispatch reserves actual wire bytes, one slot and 16 KiB downstream
before retaining a frame. Body acceptance advances the exclusive event
frontier only on last=true. The final Bytes owner drops its allocation and
slot before making S/D credit returnable; that owner also retains the pin.
One thin credit record accumulates returns, and an owned writer task sends
them in bounded frames. Done closes the body sender first and still allows
late credit returns. Channel loss grants no further credit. Fixed boundary
failure and partial-event abort reuse the existing S3a encoder path.
These adapters are not yet connected to router dispatch or qualified.

Kernel invocation records capture the optional saturation callback with the
accepted Forward. Receiving Saturated applies it before Ack, at most once for
that 429; an unacknowledged effect blocks the next Forward. The nine-effect
budget is per invocation chain. Result closes dispatch, then resolution waits
for accepted Forward and Ack settlement. Private causes remain kernel-side.
The kernel renders Direct, SaturationExhausted or FallbackOff from the recorded
cause and classifies that exact string before applying display redaction.
Worker Summary contains only counts and finish status. A disconnect completion
also requires the kernel's last joined Forward success. These records and
their controls are source-only and not yet attached to the pipe reader.

The worker runner owns one nonblocking pipe reader and one complete-frame
writer. Each admitted operation has three writer records: control/Forward,
Data, and low-priority diagnostics. A single pending command and at most one
policy invocation are allowed. Unknown identities or overlapping invocations
end the channel. Invocation repick preserves the same encoder and sequence.

Policy and emitter tasks are joined before Done. Cancellation stops new work
and closes output, while an accepted Forward or saturation acknowledgment
remains joined. A policy panic joins its emitter, sends Panicked when possible,
and ends the generation. The kernel still owns Forward cleanup and reaping;
no worker outcome clears a runtime fence. Queued inputs and assembly are
dropped before the post-Done credit wait.

The runner keeps its thin operation record until final W/S/D returns or
channel failure. Cancellation does not satisfy that wait, and Done is sent
first. The kernel adapter must send final credit on a live channel. Request
assembly has a separate conservative ceiling of six times the 64 MiB ingress
limit for JSON re-encoding. It is not an output-event or retained-history cap.
These sources and in-memory controls are UNRUN; real-child evidence is pending.

S3w currently supplies source-only wire and policy building blocks. Its
message set separates operation, invocation and Forward identities; the
kernel alone authorizes terminal encoding and classifies public failures.
Results and diagnostics contain only closed enums and numeric facts.
Public failures carry bounded UTF-8 message, type and code fields. The
kernel may retain HTTP-origin public text; local causes use fixed display.
The wire does not filter characters in upstream type or code fields.
The kernel must enforce origin before sending PublicFailure: only an actual
HTTP-origin cause may supply public text. The DTO carries no worker-asserted
origin. Local causes retain fixed display and kernel-classified type/code.
ForwardError intentionally omits cleanup: only the kernel retains joined
Forward verdicts and decides retirement. Worker Done is not a cleanup proof.

Handshake and operation text fields are limited to 16 KiB, each opaque segment to 16 KiB,
and each encoded frame to 1 MiB. Larger bodies use ordered parts.
These are transport limits, not a bound on retained history or total memory.
The worker separately bounds each buffered Forward reply to 8 MiB of bytes,
matching the integrated browser reader. It checks length before reservation
and bounds Vec capacity; a violation poisons the channel. This is unrelated
to the browser queue's 131,072 UTF-16-unit capacity. Assembly may retain many
valid replies, so this per-Forward bound is not an aggregate history limit.
Serde's internally tagged decoding can temporarily retain a content tree
alongside the input frame. Numeric-byte arrays add representation overhead;
the eventual W3 measurements must include it rather than assume a fixed
two-frame memory cost. No child, HTTP transition or retirement proof has
run for these source building blocks.

The worker emitter reserves a conservative Data frame budget before allocating
its encoded frame. Numeric byte arrays need at most four bytes per payload byte,
plus a 512-byte envelope allowance. Unused wire credit returns locally after
encoding; actual wire credit returns at kernel decode. Every retained Data item
charges one slot and a full 16 KiB decoded allocation, including empty items.
The kernel must use that same charge until the final Bytes owner drops.
These are capacity reservations, not measured allocator or total-memory peaks.

Enqueue and the worker's completed-event record share the credit-reader lock.
The started write is awaited independently of cancellation. Chunk completion
then waits on the accepted frontier, while Finish/Fail also waits for terminal
acceptance. This emitter source has no child or router qualification yet.

S3a prepares the free Responses SSE path for replacement policy workers.
It adds an in-process awaited sink and incremental borrowed event encoder.
The publisher owns accepted sequence state and irreversible content flags;
the encoder cannot declare output accepted merely by entering a job queue.
Output failures stop later dispatch and repick. A validated terminal success
survives client disconnection for guarded account-health accounting; intermediate
chunks and encoder failures cannot supply that success. At a complete event
boundary failure may emit one sequenced error;
after a partial event the body aborts without a fabricated completion.

One admitted stream holds at most one serializer job, one queued emission
command, one staged segment and two downstream segment slots. Each segment
allocates 16 KiB. Including the producer and publisher buffers, at most five
segments (80 KiB) per stream are counted. Aggregate input/history, message
composer state, metadata, thread stacks and HTTP/socket buffers are separate.
The existing operation-registration limit conservatively sizes stream
leases; HTTP, producer and host registrations retain their original meaning.
Ungated library construction has an explicit bounded default of 256 leases.

Queued Bytes retain their reservation until the last clone is disposed.
Body drop independently cancels local output and later dispatch. It does not
cancel a Forward already sent. The emitter drops staging before serializer
join, closes every data sender before its disposal wait, and leaves EOF
independent of that wait. Existing operation observers join the producer's
upstream cleanup verdict and output disposal before releasing or fencing.
No extra registration or retirement API is introduced.

A policy panic is contained by the existing completion observer. The
observer processes it when the body reaches EOF, errors or drops, then sets
the persistent unknown-cleanup fence.
Unwinding can drop the emitter's join handle; that emitter still owns its
serializer handles and output reservations. This path does not establish
joined retirement. Disposing all output can release local buffers without
clearing the unknown-cleanup fence or authorizing host retirement.

The source fixtures include P2 and P5 with one lease per admitted stream,
four stalled peers while a fifth completes, retained clones across EOF/drop,
pending Forward cleanup, encoder errors and actual HTTP handler consumption.
They report allocation-capacity counters and unpaused milestones separately
from process memory samples. Main qualified the 27 affected correction cases,
five repetitions of each profile and 21 focused contextual integration cases.
The original 351 passes and five failures remain separate; the corrected
361-case inventory was not rerun in full. P5/P2 median direct-sink progress
was 1.019 across the fresh repetitions, a diagnostic rather than a threshold.
Blocking-pool contention, stack/RSS cost and transport retention are still
operational limits, with no total-memory or client-receipt guarantee.
Direct in-process BodyExt::collect consumers must copy and release frames;
remote clients buffer different allocations and need separate evidence.

S1 began on `refactor/kernel-hot-swap-20261026`, based on qualified `c8371bf`,
and is now integrated on the experimental feature branch. The activation command
still stops and restarts the application; it does not preserve active streams.

The browser application currently owns its HTTP listener, account admission,
private session-store lock, concurrency history, state database and browser
bridge. The Python host owns browser processes, page work and the rate journal.
The native application has its own request, session, device and worker owners.
Starting a second complete application would contend with these owners.

S1 gives the serving policy ownership of workspace saturation and selection
cursors, and gives the client transport ownership of its authorization caches.
The application shares one account runtime through request and host cleanup.
That state must remain shared across later policy workers. `Session` contains
the private cookie and has no
serialization implementation. A process adapter must use request-bound opaque
handles rather than serialize the current credential-bearing port objects.

An update boundary must retain one state owner and pin each accepted request
to its application generation through response delivery and external cleanup.
A response body ending is not proof that browser or worker cleanup completed.
Unconfirmed cleanup must retain the current admission fence.

The intended first proof is an offline generation transition: A continues an
accepted stream while ready B accepts new requests. Failed readiness leaves A
active. Rollback changes future admission without moving accepted work.

The agreed sequence is S1 owned state, S2 an in-process authorized forwarding
port, then S3 a free-Responses policy worker. S1 has 351 isolated and 37 focused
integration checks passing. The S2a legacy baseline at `d22d2b5` now passes
19 new and 146 retained checks, including the explicit agents-category case.
S2b extraction now passes 266 isolated and ten integration checks. S3 still
requires implementation, review and execution. The
runtime retains credentials, admission and cleanup ownership;
the worker owns request policy and rendering. Existing 429 callback points
must remain unchanged. S3 may acknowledge only a runtime-recorded 429 once.

S2b's qualified in-process integration places conversion, Responses policy, data limits
and the shared cooldown helper in `framer-protocol`. The runtime binds the
complete Forward unit per invocation and passes owned reply bytes. Compatibility
re-exports preserve existing native imports and Messages behavior. This creates
a dependency boundary. Process replacement, its offline generation-transition
proof and full live acceptance remain open.

The later lossless worker transport must count downstream capacity waits in
the existing 300-second active budget, which excludes only local cooldowns.
That budget prevents later dispatch; it does not abandon already-sent work.
A slow client can therefore exhaust the budget where the legacy callback
currently drops frames. S3 must test and document that visible difference.

Every transition needs explicit protocol and state compatibility, bounded
transport, cancellation, process-exit observation and retained rollback state.
Kernel upgrades and incompatible state migrations require a separate drain.
Process isolation does not imply survival of a worker crash or forced kill.

The integrated source retains the qualified sidecar page-acquisition,
page-use and device-cleanup corrections. Later stages must preserve their
bookkeeping and finalizer behavior. A worker failure may send only eligible
new requests to legacy handling through the shared fences; accepted work
must never be replayed, and unconfirmed cleanup still prevents admission.

CI currently runs on pushes to the `diddletime/framerize` build fork. It builds
both applications with locked dependencies and runs offline/package fixtures.
Upstream PRs do not trigger a duplicate build. A successful local fixture run
does not establish hosted CI success or live account acceptance.

The child transport has one owning reader and writer. Its writer reserves a
fixed number of producer records; a record permits one encoded allocation,
capped at 1 MiB plus the prefix. Ready producers rotate between complete
frames. Operation cancellation cannot retract an accepted frame. Generation
teardown poisons the channel before aborting and joining its pipe tasks.
These transport bounds do not establish per-operation W/S/D accounting.

Worker launch currently requires Linux with close_range CLOEXEC support
(kernel 5.11 or newer). It hashes and executes the same open ELF descriptor,
keeps that descriptor above stderr, clears environment variables and marks
all other inherited descriptors CLOEXEC. A syscall failure refuses launch.
Immutable inode contents remain a publication requirement. The process owns
its direct child; this is not a sandbox for executables spawning descendants.
Stderr is discarded through 4 KiB scratch. Counts cover observed bytes only;
bytes still unread at teardown are unknown. OS errors retain a closed stage
and numeric errno, without executable paths or raw error text.

The optional manager reserves Starting before launch and retains staging,
retirement and shutdown through caller cancellation. It accepts one exact
Hello, completes the Ready write, removes the settled writer slot, then
checks process liveness under its lifecycle lock before publication.
Activation and pin selection share that lock. Old Ready workers stay eligible
for rollback until explicit retirement. At most two slots exist, including
unconfirmed tombstones. Dead workers cannot become eligible through stale Ready.

Retirement first excludes selection, then waits for final pins, drains the
writer and joins the child. A final pin without explicit confirmation keeps
the generation unconfirmed. The adapter must confirm only after worker Done
or joined channel loss, local jobs and kernel Forward verdicts, and must put
pin clones in every final Bytes owner. That adapter is still incomplete.
Forced error shutdown reaps even with pins outstanding and retains the fence.
Drop requests teardown only; library callers must await manager shutdown.
The browser app now does so after application drain and before host shutdown.
Semantic credit, Forward verdict attachment and real-child proofs remain
unfinished. Synthetic process outcomes do not establish reaping.

The manager logger has one bounded inbox and one joined blocking append job.
Each queued record has fixed-size kernel identifiers and closed numeric data;
request correlation is captured under CURRENT before dispatch. Full inboxes
increment a saturating counter. Worker drop reports log only increases and
mark MAX as a lower bound; channel loss reports the remaining count unknown.
Neither counter covers silent disk-append errors. The existing core log-open
warning is unchanged. NDJSON fixtures run individually in fresh processes.

## Reference evidence

These historical revisions were inspected, rather than inferring quality from
repository age. Current documentation is listed separately.

- [Tokio 1.14, 2021-11-16](https://github.com/tokio-rs/tokio/tree/884a9a4b1800219dee696ce9bf8eb8fe93b92f71): `sync/mpsc` and its tests distinguish closing admission from consuming remaining messages. Bound bytes as well as message count; a channel cannot attest browser cleanup.
- [Redis 6.2.6, 2021-10-04](https://github.com/redis/redis/tree/4930d19e70c391750479951022e207e19111eb55): command admission and `tests/unit/quit.tcl` test that buffered work after QUIT is not executed. Its single-threaded command model should not become a global lock around long streams.
- [ripgrep 13, 2021-06-12](https://github.com/BurntSushi/ripgrep/tree/af6b6c543b224d348a8876f0c06245d9ea7929c5): executable glue composes focused libraries. It supplies a code organization example, not a service update protocol.
- [Cargo 0.57, 2021-10-04](https://github.com/rust-lang/cargo/tree/4ed5d137baff5eccf1bae5a7b2ae4b57efad4a7d): the integration suite groups concurrency, process death and compatibility behavior. Test organization alone is not proof of coverage.
- [nginx 1.20.1, 2021-05-25](https://github.com/nginx/nginx/tree/98a892514206ee63d418d9f355c92b6d4ce6113a): process-cycle reload retains the old configuration on failure and drains old workers. Its 100 ms startup pause is insufficient readiness evidence for Framerize.
- [Hyper 0.14.15, 2021-11-16](https://github.com/hyperium/hyper/tree/d0b1d9ed3a10013ab356bc7d9b283e179857a672): shutdown and drain watchers separate notification from completed futures. External resource cleanup needs additional ownership.

The current [Tokio shutdown guide](https://tokio.rs/tokio/topics/shutdown)
separates detection, notification and waiting. The
[nginx control guide](https://nginx.org/en/docs/control.html) distinguishes
configuration reload from executable upgrade. Both were read on 2026-10-26.
[OTP 24 release handling](https://www.erlang.org/docs/24/design_principles/release_handling)
requires explicit upgrade/downgrade paths and warns about suspension windows;
Rust has no equivalent VM hot-code contract. [SQLite testing](https://www.sqlite.org/testing.html)
motivates targeted I/O, crash and compound-failure tests, not copying its scale.

[matklad's 2021 architecture guide](https://matklad.github.io/2021/02/06/ARCHITECTURE.md.html)
supports a short boundary map separate from historical evidence.
[Spolsky's 2000 rewrite essay](https://www.joelonsoftware.com/2000/04/06/things-you-should-never-do-part-i/)
supports retaining accumulated behavior through incremental replacement.
Neither article establishes correctness of a proposed Framerize design.
