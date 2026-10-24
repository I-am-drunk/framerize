# Current V2 structural discriminators, 2026-10-24

Source milestone on `goal/browserless-fingerprint-structure`, based on
`71b6326`. The runtime owner reports all 16 comparator cases **PASS**,
17,321,984 bytes peak memory, zero swap/events and the unit absent afterward.
Tests remain **UNRUN in this source checkout**. Only source inspection,
static constant derivation and diff review ran here.
All tests, builds, SDK/browser execution and private/runtime work belong to
owner `544711e3-0b76-4cb4-9732-124a0486847e`. Shared status docs stay with that owner.

Owner-reported R14: normal and paired browser requests returned HTTP 200 with
output; dedicated V2/native returned a complete Castle 403; cleanup confirmed.
The passive browser baseline observation has 104 fields and consumes 800 bytes.
The earlier socket-denied synthetic SDK observation has 88 fields and 563 bytes.
Root reports 17 browser-only fields, one synthetic-only field and 12 changed
shared fields. These are different observations, **not a paired capture of
R14's browser and R14's SDK**. No private inputs were read in this source lane.
No further live retry is queued. These findings do not establish acceptance.

## Source binding and scope

Mappings below use only the saved public Framer V2 factory with SHA-256
`df5a150be914e7d267126f35de92934993e7930f18c3fd00bf79a86bd788ec58`.
Offsets are zero-based positions in that exact source. The existing
[serializer record](v2-serializer-compatibility-20261024.md) binds wire 13,
groups 0/4/7/8/9 and the current parser. Its historical REA evidence covers
group/serializer slices; it does not independently prove these field meanings.
The new mappings are static source derivations, with no target execution.

The existing read-only constant mapper resolves `t[P8]=13`, `t[U8]=15`,
`t[$6]=23`, and the aliased `t[R8]=12` assignment at 383106. The group getter
`r[cO]` reads `t[Q1]` into the third serialized group, whose wire ID is 7.
No field label from an older public decoder is assumed to survive unchanged.

## Four reported field differences

**7/13: synthetic type 2/length 1, browser type 1/length 1.** The call near
500520 passes `r[fE]` to `r[sl]` with explicit-false inclusion enabled.
`r[fE]` (407062) returns early with false unless its engine discriminator
matches. Otherwise it appends a sandboxed iframe (`allow-scripts`), tries to
read `contentWindow.navigator`, records success/failure, and removes the frame.
`r[sl]` (458960) emits type 2 for truthy input and type 1 for explicit false.
The tags distinguish branches without any decoded payload. Type 1 cannot
distinguish the early discriminator return from a failed property read.
The report does not establish which path occurred or why the environments differ.

**7/15: synthetic type 4/length 42, browser type 3/length 2.** `r[ME]`
(408310) captures an exception from evaluating/calling the canvas 2D prototype's
`getImageData` with an undefined receiver, including a missing-global failure.
The collector at 489118 compares that message with `t[yl].Jf`:
a known entry uses its one-byte index (type 3);
otherwise it uses the encoded-string path (type 4). The tags distinguish
indexed versus string representation, not the message or indexed value.
The type-4 length includes its header, length byte and transformed payload.
It must not be reported as the original message's character count.

**7/12: browser-only type 7/length 56.** `r[eE]` (407603) reads the first
`performance.getEntriesByType('navigation')` entry and produces nine timing
quantities. The collector near 487450 sends those through `r[Kb]` (464771):
one count byte and nine six-byte quantities, plus the field header, total 56.
Presence supports that encoded branch; no timing values survive projection.
Absence cannot distinguish unavailable API, missing entry or failed collection.

**7/23: synthetic-only type 7/length 7.** `r[jE]` (426774) times repeated
canvas operations with `willReadFrequently` true and false, forms a ratio,
and rounds it. The collector near 490900 writes only a truthy result through
`r[gh]` (464531), yielding six payload bytes plus the header. Absence can mean
an unmet prerequisite, collection failure or zero result. The fixed width
cannot reveal the ratio, timing quality or reason for omission.

These mappings guide diagnosis of genuine behavior only. They provide no
evidence for spoofing identities, overriding APIs or weakening acceptance.

## Remaining ten shared changes

The following owner-supplied tuples are `(type, byte_length)`, ordered from
the prior synthetic SDK observation to R14's accepted browser baseline.
They add no R14 SDK observation or causal attribution. All names and offsets
below come from the same pinned factory, not a historical field-name table.

**0/0: (4,2) to (3,2), navigator platform.** `r[Hs]` (183776) reads
`navigator.platform`; its collector at 483801 selects an index from the
current platform list or a transformed string when absent from that list.
The reported types distinguish string versus indexed representation.
The string-side length includes zero encoded payload bytes, which does not
establish the original unnormalized property value or the browser's index.

**0/1: (3,2) to (4,2), navigator vendor.** `r[ew]` (183818) reads
`navigator.vendor`; collector 484017 uses its own two-entry lookup list with
the same index/string fallback. This changes representation in the opposite
direction from 0/0. Neither index nor original vendor text is in the report.

**0/4: (7,5) to (7,7), screen dimension pairs.** The collector near 484823
encodes `(screen.width, screen.availWidth)` and
`(screen.height, screen.availHeight)` through `r[l3]` (462214).
Its helper compares masked integers: the first is masked to 15 bits, the
second to 16 bits. Equal pairs occupy two bytes, unequal pairs four bytes.
Including the field header, lengths 5 and 7 distinguish two compact pairs
from one compact pair. They do not identify the unequal pair or actual sizes;
the readers also replace falsy properties with zero.

**0/12: (7,75) to (7,83), navigator user agent.** `r[Dd]` (183902)
reads `navigator.userAgent`. Collector 486359 supplies it to the bounded
long-string encoder `r[h7]` (180002), with a source cap of 512 bytes.
The observed lengths differ in the encoded representation of that property.
They do not reveal the UA text or establish its agreement with HTTP identity.

**0/29: (3,2) to (4,50), generated runtime error message.** `r[ng]`
(380468) deliberately accesses a property on an absent array element and
returns the caught exception's message. Collector 497441 chooses a known
message's index or the transformed-string fallback. This is an engine-error
representation change; it does not reveal the message or identify a 403 cause.

**7/18: (7,5) to (7,7), window dimension pairs.** Collector 490599
encodes `(innerWidth, outerWidth)` and `(innerHeight, outerHeight)` through
the same `r[l3]` helper. The 0/4 compact-pair interpretation and masking limits
apply, but these are window dimensions, not the screen properties in 0/4.

**8/24: (4,38) to (4,54), Intl prototype-error diagnostic.** `r[ET]`
(447388) attempts `Object.setPrototypeOf(window.Intl, window.Intl)` when
`Intl` is present. On error it takes the first stack line, limited to 50
string code units; other paths retain fixed absent/no-error markers.
Collector 491476 encodes every returned variant as a transformed string.
The shared type cannot distinguish those branches or recover the stack text.

**8/28: (5,3) to (5,2), enumerable window-key count.** `r[cF]`
(457824) obtains `window.Object.keys(window)`, then returns its count and a
separate hash. `r[QF]` sends the count to field 28 at 471830. The width change
distinguishes the two packed-integer size classes after encoder clamping.
It does not reveal the exact count, key names, hash or capability set.

**9/0: (4,10) to (5,2), navigator getter-error aggregation.** `r[pT]`
(445778) attempts to obtain selected navigator prototype getters and invoke
them with an invalid receiver. It collects exception stacks, including lookup
failures, counts distinct captured stack values and compares selected results
to a reference. The collector at 492283 encodes
the structured result's count as type 5; the fallback at 492600 uses type 4.
The reported tags distinguish those output paths, not stack contents or the
number of successful getters. The count is not a count of available APIs.

**9/1: (4,6) to (4,34), companion getter-difference list.** On the same
structured path, collector 492468 serializes the property-name list whose
captured stacks differ from the selected reference. The fallback at 492756
uses a fixed marker instead. Both are type-4 strings. Read this jointly with
9/0's branch: the lengths do not reveal which property names, how many names,
or which errors appeared, and length alone cannot identify the fallback.

## Additional encoder limits

For the 7/15 type-4 path, `r[p5]` (179104) first applies `r[Wd]` (165453):
surrogate pairs become spaces, selected controls are removed, and whitespace
becomes spaces. It then encodes at most 251 UTF-8 bytes for this salted path.
`r[ZM]` (170372) packs four-byte words with zero-filled unused bytes; `Ph`
(87409) preserves the word count; `r[Hy]` (170188) expands every word to four
bytes. The field's structural length is therefore `2 + 4 * ceil(L / 4)`,
where `L` is the normalized, bounded UTF-8 byte count, not the original text.
The reported length 42 constrains `L` to 37..40. It identifies neither the
message nor its original character count. No payload was decoded to derive this.

For 7/23, `r[jE]` requires a callable `performance.now` and a defined
`HTMLCanvasElement`. Each measured helper still guards canvas/context creation
and can skip its drawing loop. Timing surrounds the helper, so a nonzero ratio
does not independently prove that drawing happened. A zero duration on either
side produces zero; other ratios are rounded to three decimal places before
the collector's truthy-only emission. An omitted field therefore does not
separate zero/rounded-zero timing from an unmet prerequisite or a caught error.
This is a source-level limitation of this field as a capability discriminator,
not a measurement of either R14's browser or its isolated worker.

## What the comparison can and cannot distinguish

`compare_structures(baseline, native)` accepts the existing projection shapes.
It reuses `_structure` for bounded validation and compares fields by
`(group_id, field_id)`. Changes retain only canonical keys, integer type/length
facts and absent-side nulls. Input dictionary keys are never forwarded.
No Boolean meaning, payload, token, error text or identity is decoded/emitted.
It accepts `None` as unavailable and reports `comparable=false`, with an
unknown match result. Two available empty shapes are distinct from that case.

Presence changes and type changes are visible even when counts and totals
match. Opposing per-field length changes remain visible when the total is
unchanged. Type 1 versus 2 already carries Boolean branch information;
therefore "no decoded payload" does not mean "no semantic information".
An absent field is not a false Boolean or proof that an API is absent.

Equal-width payload substitutions are invisible for types 3/4/5/6 and fixed
raw layouts. Type 5 distinguishes one-byte versus two-byte packed encoding,
not the value. Bitsets expose only `ceil(bit_count/8)` through their length;
both bit values and counts within a bucket remain indistinguishable.
Dimension-pair lengths distinguish the number of compact pairs but cannot
identify which pair was compact when the two orders have equal length.
Array widths constrain layout, not element values. Transformed strings may
also collide in length despite different original byte or character counts.

`structural_match=true` means only that the projected field maps match.
This is not token validation: the projection contract is broader than the
wire parser's contract, and field order within a group is not compared.
The helper does not inspect frames, event/trailer bytes or upstream responses.
The caller retains source-version and observation pairing, lease/account,
identity/egress, freshness, cooldown and joined-cleanup checks. No result
grants minting, dispatch, retry, provenance or acceptance authority.

## Qualification handoff

Exact owner-only fixture entry:
`python3 -B reference/fixtures/v2_structure_compare_tests.py`.
Sixteen synthetic cases cover unavailable/empty inputs, Boolean-tag changes,
unchanged totals with changed fields, group identity, equal-length payload
collisions, bitset buckets, dimension-pair ordering, packed-integer widths,
detached output, malformed-input privacy and canonical output keys.
They reuse the existing parser and do not create a second decoder.

The frame/parser/projection/observer qualifications documented in
`docs/development-status.md` remain prior owner evidence for unchanged code.
The owner now separately reports the 16 comparator cases passing; no test or
runtime execution occurred in this source lane. Subsequent changes here only
extend the source note and reuse that qualification for unchanged tooling.
Next action: the runtime owner uses the mapped differences to choose the next
bounded discriminator. Any future matched observation or runtime change stays
with that owner. The shared goal remains a fresh valid mint and accepted native
inference with all retained guards.
