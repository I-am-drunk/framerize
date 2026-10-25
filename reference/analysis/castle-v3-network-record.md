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

## Confirmed unavailability

`unavailable_network_record` matches the no-argument `xP()` call at
95131..95337. It retains the 56-position sparse row, empty address and summary
strings, empty candidate/state arrays, and `UP(["unavailable", ""])` at field
47. The no-argument fallback is used for unavailable networking in `N_`;
it is distinct from the normal finalizer receiving an empty observation.
The API does not implement `xP(n)` with a supplied argument or decide when
networking is unavailable.

The existing record comparison extracts `xP` and its literal bindings from
the same pinned source. All 32 full-record comparisons and the non-scalar
refusal check pass in three tests, with zero skipped or filtered. The added
test also requires the unavailable and ordinary empty records to differ.
The isolated check starts no SDK module, browser, account or network request.

## Typed observation reduction

`NetworkEvents` now implements the preceding pure transformations using
supplied observations, including the shared arrival-order signature set.
The source anchors are `od` (95337..95470), `IP` (95470..95593), state
reduction (5982..6044), candidate normalization (356056..358548), candidate
insertion (358549..358596), and the SDP callback within 359062..360014.
The reference harness extracts the original functions, dispatchers, computed
method selection and literal bindings. It does not evaluate the SDK module.

Structured type/protocol/address properties select the structured branch;
otherwise the raw candidate text supplies its fields. Address classification
is a syntactic category, not a valid-address check. Numeric port classification
preserves fractional, nonfinite and above-65535 behavior. Signature collisions
are deduplicated by their joined string before the 16-record cap. SDP entries
use that same set. State histories preserve first-seen lowercase order.

`scripts/check_v3_network_events.py` compares 56 full event sequences plus
32 address and 13 numeric-port cases. All 57 tests passed in isolation.
The failed predecessor exposed a native trailing-newline classification bug;
the correction retains the source regex's exact end-of-string requirement.

The API receives typed RTC properties and numeric ports. It does not invoke
getters, recreate arbitrary JavaScript coercions, open network connections,
or establish callback/finalization timing. Non-ASCII lowercasing also depends
on the toolchain and source runtime's Unicode tables. The checked reference
uses Unicode 16.0. Real collection, compatible collation, service acceptance
and total application memory remain unverified.
