# Native collection handoff

The experimental V3 worker collects with the pinned SDK and returns those
values through its existing private pipe. Rust consumes that collection and
produces the token returned by the request's minter. This is a collector and
encoder split, not complete native collection or a measured memory saving.

## Source boundary

SDK SHA256 `3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71`
is checked before replacing bytes `[274980,281215)` with one collection call.
Configuration, request collection and preceding Promise waits are preserved.
The removed expression contains serialization, compression and the envelope.
The instance accepts exactly one collection of 765 positions. Each worker
creates a fresh instance, so this does not rely on cross-request queue behavior.

The waits establish fulfillment of their captured handles. They do not prove
completion of unlisted underlying activities or a later configuration.
The handoff copies plain data synchronously at the serialization boundary.
Accessors, proxies, non-array objects, cyclic arrays and inherited array values
are refused rather than evaluated or replaced with invented observations.

## Private protocol and ownership

`FCLT`, version 1, three zero bytes and a big-endian payload length prefix one
JSON document. The document has version 1 and a tagged array. Tags preserve
undefined, null, booleans, numbers including negative zero and nonfinite values,
and strings as UTF-16 code units. Holes normalize to undefined, as in the
serializer. No raw collection data enters diagnostic metadata.

The producer and consumer bound payloads to 1 MiB, collections to 4,096 total
values, individual arrays to 2,048 entries, and strings to 65,536 total UTF-16
units. The consumer requires all 765 top-level positions and rejects old token
frames, unknown fields, duplicate fields, trailing bytes and malformed values.
These are supported-input limits, not claims about every possible SDK output.

Success still requires the exact V3 branch, unchanged sources, the original
deadline, a collection handoff age of at most two seconds, and worker cleanup.
The parent accepts the collection only after metadata and child exit succeed.
It performs native encoding in a blocking task that the same owner joins.
Cancellation and deadline are checked again before publishing its result.
The original pre-spawn freshness bound remains; encoding does not renew it.
The envelope's wall-clock sample occurs after serialization and compression.

V2 continues to use `FSDK` token frames and its separate metadata validator.
Neither path treats a local output as evidence of upstream acceptance.

## Qualification

Fourteen focused JavaScript checks, 85 Rust worker checks (including three
decoder checks), and four V3 saved-device ownership checks pass. The fixture preserves
undefined, null, negative zero, nonfinite numbers and lone UTF-16 surrogates.
It compares serialization and fixed-clock encoding with the expected values.

Selections: `worker::` excluding `worker::backend::tests::device::v2::`, then
`v3_authority`, plus the JavaScript collection, handoff and value-capture files.
V2 process/metadata tests remain in the selected worker group. A stale fixture
pin and permissive null-tag parsing were corrected during qualification.

On 2026-10-25, one isolated synthetic SDK operation completed through the
production collection pipe and native encoder. The owner completed in
1,936 ms, the worker was joined and absent, and the output was discarded.
The unchanged limits were 128 MiB, no swap, one CPU, 32 tasks and 40 seconds,
with private network/filesystem views and the worker's syscall filter.
The first harness layout failed to mount a new module before SDK startup;
the corrected mount layout passed with all source and binary pins unchanged.

## Fresh frame comparison

The snapshot probe now copies the actual pre-serialization values with the
production collector and writes its `FCLT` frame through a private stdout pipe.
Its instrumented loader retains the original SDK encoder, recording the
original serialization, clock and output digest from that same operation.
The parent drains the pipe, waits for the child and preserves the frame in
private evidence. The normal loader and service are unchanged by this probe.

`compare-v3-collection` compiles the original Rust decoder and encoder source.
It checks that the decoded frame contains the captured values, reproduces the
serialized bytes, and matches the original output digest and length using
the original clock. It discards the native output. Seven negative checks
reject a damaged header, truncated frame, nonsynthetic snapshot, mismatched
values, changed serialization, changed output digest and changed clock.

On 2026-10-25, the qualified fresh operation matched all 765 positions,
5,411 serialized bytes and 5,297 final bytes. Its 37,614-byte frame crossed the
pipe; the child exited and the first cgroup observation contained only the
parent. No memory-limit event or swap use was recorded. The comparison and
all seven negative checks passed. Three existing capture checks also pass.

An earlier capture also matched, but its immediate cgroup check failed.
That harness did not record the remaining members, so its cause is unknown.
The follow-up records membership and allows at most two seconds for the group
to contain only its parent. It needed no wait in the passing operation.
The earlier failure remains separate evidence, not a passing cleanup result.

Evidence: investigation directories `fresh-collection-comparison-r1` and
`fresh-collection-comparison-r2` in the owning thread's private scratch tree.
The SDK identity above and 1,268 input pins bind each capture; the comparison
also binds its native source and executable. Snapshot schema 4 carries the
same tagged values as schema 3 and adds the separately preserved frame.

Build the checker with `cargo build -p framer-native --example compare-v3-collection`.
It takes `FRAME SNAPSHOT REPORT` paths and exclusively creates the report.
Capturing a new SDK operation still requires the existing isolated harness.
This is synthetic, instrumented equivalence. Collection still uses SDK/JSDOM.
Live acceptance, request memory savings and sustained operation are unverified.
