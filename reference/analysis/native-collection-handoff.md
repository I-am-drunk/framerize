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

The earlier direct-input comparison matched one synthetic SDK operation's
765 positions, 5,412 serialized bytes and 5,297 output bytes. That saved result
is retained separately; no byte comparison with the original encoder was
performed for this new operation. Collection still uses SDK/JSDOM. Live
acceptance, request memory savings and sustained operation remain unverified.
