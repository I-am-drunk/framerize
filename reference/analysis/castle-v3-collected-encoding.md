# Post-collection encoding boundary

Source: `reference/decompiled/castle/castleSDKV3-LNGA36I3.mjs`, SHA256
`3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71`.
Byte offsets are zero-based and half-open.

The call beginning at 274980 serializes the shared `Q` array, passes its
bytes to `$2n`, and chains the final envelope transformation. Serialization
is at 274985..276393, the compression implementation at 575295..581410,
and final encoding at 276403..281222. `encode_collected` connects the
previously compared native implementations at that same boundary.
It deliberately chooses the SDK fallback compressor. A platform compressor
may emit different valid compressed bytes.

`IZ` (210755..281222) waits on readiness and asynchronous collector work
before final serialization. Configuration also writes to `Q`. Static parsing
finds one `Q=[]` declaration, no function parameter named `Q`, and 766 direct
`Q[...]` assignments, including index 764. This count excludes indirect alias
writes and is not a schema, a completeness check or a count of runtime results.
The native API therefore accepts the supplied array as-is, without inventing
a fixed-size input, zero-filled observations or readiness evidence.

The comparison exercises 17 supplied arrays through the original serializer,
fallback compressor and envelope. Three are synthetic 765-position arrays
covering positional data, undefined entries and repeated values. All match.
Invalid timestamps retain the envelope's explicit refusal. Earlier component
results remain valid and were not rerun as a broad suite for this change.

Producing the completed values from genuine observations, matching runtime
ordering, integrating the serving worker and measuring total memory remain
unfinished. A matching encoded string from supplied data proves encoding only.
