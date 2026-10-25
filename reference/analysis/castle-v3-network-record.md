# Native network-record assembly

The reference is `reference/decompiled/castle/castleSDKV3-LNGA36I3.mjs`,
SHA256 `3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71`.
Offsets below are zero-based, half-open byte ranges. Intake's source-only
handoff is bound by SHA256
`762647e8ff44546416c8972d5a79e7ef301fe9b5055f62b38e129796800be038`.

The pure finalizer inside `N_` (354907..355601) selects the first 16
candidate records before sorting their signatures with `localeCompare`.
The signature dispatcher (12150..12263) joins seven fields with colons.
Protocol and type/protocol summaries use separate default UTF-16 key sorts.
The metrics array contains protocol summary, signatures, grouped counts,
connection history, gathering history and signaling history. Each history
is sliced to eight entries. `UP` (95593..95721) hashes its JSON text.
The final sparse record retains the selected raw candidates and both supplied
addresses along with those summaries, histories and the metrics hash.

`assemble_network_record` implements that pure boundary. It accepts scalar
candidate fields and an explicit signature comparator. It does not perform
the earlier candidate deduplication or lowercase/unique state reduction.
The caller must supply the observed values and matching locale collation.
It preserves the normal object's special `__proto__` protocol-key behavior.

`scripts/check_v3_network_record.py` compares serialized complete records
with the extracted original finalizer on 31 synthetic inputs. The isolated
run passed those comparisons and a separate non-scalar refusal test, with
zero ignored or filtered tests. No full SDK, browser or network was started.
This proves the tested packing behavior, not real observation collection,
portable collation, upstream acceptance or a reduction in application RAM.
