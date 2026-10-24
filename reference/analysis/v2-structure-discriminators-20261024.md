# Current V2 structural discriminators, 2026-10-24

Source milestone on `goal/browserless-fingerprint-structure`, based on
`71b6326`. The new comparator and its synthetic fixtures are **UNRUN**.
Only source inspection, static constant derivation and diff review ran here.
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
`docs/development-status.md` remain prior owner evidence for unchanged code;
they do not qualify this new comparator. All new fixtures remain **UNRUN** here.
Next action: the runtime owner qualifies this exact fixture entry before
using the helper on existing projected facts. Any future matched observation
or runtime change remains the owner's separate decision. The shared goal is
still a fresh valid mint and accepted native inference with all retained guards.
