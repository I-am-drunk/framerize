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
shared fields. All 30 differing rows in that comparison have mappings below.
These are different observations, **not a paired capture of
R14's browser and R14's SDK**. No private inputs were read in this source lane.
No further live retry is queued. These findings do not establish acceptance.

## Rust composition for the native observation gap

The source port on `feat/resident-token-structure`, based on `be4ff76`, adds
only `resident_token_structure.rs` and its adjacent test module, plus this note.
Its `describe(&str) -> Option<Structure>` exports field structure and consumed
length only. It preserves canonical base64url, V13 framing, XXTEA, both nibble
transforms, all 36 raw layouts, and the strict `consumed < inner.len()` check.
The final `ff` must remain outside the consumed groups; event contents are not
parsed. Input is capped at 16,384 bytes and output at 155 entries.

All 19 Rust cases are **UNRUN**, including the saved synthetic frame vectors,
complete synthetic compositions, malformed inputs and checked-index boundaries.
Static source comparison confirms all 36 type-7 keys and 15 fixed widths match
the qualified Python source. No Rust build, test, SDK or private capture ran here.

R15's reported browser 200/native 403 outcome still lacks a native projection.
Next action belongs to the runtime owner: qualify these cases and integrate the
borrowed-token observer in its explicit resident SDK path. Parent wiring, lease,
deadline, same-device completion and live execution remain owner-held.

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

## Remaining sixteen browser-only rows

These owner-supplied rows complete the 17 browser-only fields, including 7/12
above. They compare saved synthetic result `4a5cf5e6` with R14 browser baseline
`472150ef`, not R14 SDK data. No comparison was repeated and no payload decoded.
Tuples below remain `(type, byte_length)`; source meanings do not add value
output to the comparator or establish the reason for an observed omission.

**0/9: (7,6), MIME-type inventory; 0/10: (7,6), plugin inventory.**
`r[lw]` (184820) reads `navigator.mimeTypes` and takes each entry's `type`.
`r[sw]` (185145) reads `navigator.plugins` and concatenates each entry's name,
description, length and filename. Collectors near 485450 and 485750 encode
nonempty arrays as a count byte and four-byte hash of sorted, joined entries.
The six-byte field includes its header. Empty arrays or failed collection
produce no field; presence reveals neither the count nor individual entries.
The equal widths of these two different inventories establish no equality.

**0/14: (7,3), media-device kind flags.** `r[rg]` (375481) conditionally calls
`navigator.mediaDevices.enumerateDevices`. Its three flags distinguish video
input/video, audio input, and audio input/audio; collector 482577 reverses the
source array into that order and bit-packs it. No device IDs or labels are
used. The helper initializes three flags before awaiting the result, so a
rejection, empty device list or later failure can still emit the same shape.
An unmet API/engine guard or earlier failure can omit it. Presence does not
prove successful enumeration, permissions or available hardware.

**0/23: (3,2), navigator build ID.** `r[bw]` (186346) reads
`navigator.buildID`; collector 495964 uses a lookup with a string fallback.
The reported type identifies the indexed representation. Absence is
compatible with an unavailable value; the tag is not browser attestation.

**4/19: (7,6), CSS selector probe groups.** `r[dT]` (439296) groups the
hardcoded selector probes from `r[eT]` (436336). `r[lT]` (439980) creates
elements, appends them, checks `offsetParent` asynchronously, then removes
the container. A group matches if any of its probe elements lacks that parent.

The call at 502019 sends matching group indices through `r[NF]` (459985),
which emits a count byte and four-byte hash only for a nonempty list.
Absence cannot distinguish no matches from failed/incomplete collection.
Layout behavior can affect the predicate; this is not proof of an installed
content blocker, and neither matching indices nor their count are projected.

**7/20: (3,2), screen orientation type; 7/21: (5,2), orientation angle.**
`r[xE]` (430194) reads `screen.orientation.type` and `.angle`; the collector
near 490000 uses an orientation-name lookup/string fallback and a packed
integer, respectively. Missing orientation or undefined/null properties can
omit fields; zero angle is included. These tags reveal no angle or direction.

**7/24: (4,6), default voice language; 7/25, 7/26 and 7/27: each (5,2),
voice counts; 7/28: (3,2), voice-derived platform category.** `r[_E]`
(424112) requires `speechSynthesis`, callable `getVoices` and the
`onvoiceschanged` property. It collects immediately and again on that event.
`r[Zb]` (423321) supplies the fields to collector 480464 onward:

- 24 takes the default voice's language prefix before the first hyphen.
- 25 counts all voices; 26 counts voices whose `localService` is truthy.
- 27 counts voice names containing the source's `Google` substring.
- 28 uses `r[ZE]` (422464) to classify voice names against ordered patterns.

An empty voice list can still emit zero counts while omitting language and
category. No default voice can omit 24 independently of the counts.
Missing APIs or collection failures can omit the whole set. Type 3 identifies
the category lookup path, not the host's actual platform; equal packed widths
do not establish equal counts, and the language text remains unobserved.

**8/12: (2,1), ordinary/offscreen canvas serialization comparison.** `r[TE]`
(413707) draws the same shapes/text into ordinary and offscreen 2D canvases,
copies the offscreen result to another ordinary canvas, then compares their
`toDataURL()` strings. Collector 488682 enables explicit-false inclusion.

Type 2 therefore exposes the truthy branch without decoding a payload.
An unmet prerequisite or caught failure can omit this field; absence cannot
be reported as the other Boolean branch. The comparison does not establish
correct rendering, identical runtimes or upstream acceptance.

**8/13: (5,3), canvas image round-trip difference sum.** `r[EE]` (411556)
draws a 200-by-200 canvas, assigns its data URL to a bare `new Image`, then
compares the original pixel array with the image redrawn on a second canvas.
It returns the sum of absolute per-channel differences; collector 480283
encodes that sum as a packed integer. Zero is included when returned.

Missing canvas/context/pixels, failed construction, no observed load callback
or unequal pixel-array lengths can prevent a numeric result. The three-byte
field identifies the larger packed width after clamping, not the sum or pixel
contents. This source trace neither observes a constructor failure nor proposes
an override; constructor qualification remains with the runtime owner.

**9/9: (4,82), SharedWorker reply slot 2; 9/19: (4,6), reply slot 12.**
`r[YE]` (431897) handles a shared worker's message port. Its callback near
477612 accepts report discriminator 2 and passes slot 2 to field 9; collector
478991 passes slot 12 to field 19. Both use transformed-string encoding.

The inline producer selected by `r[sF]` (473251) reads the worker navigator's
`userAgent` into slot 2. It sets slot 12 to a fixed marker only on its branch
without `navigator.userAgentData`. Missing/failed worker setup, missing report
or null/undefined slot values can omit the corresponding fields.
The same factory can instead select a configured external worker URL. The
configuration path at 550931 copies a non-null/undefined own `swUrl` option
to internal `ou`; the branch at 473691 uses that option when truthy and
otherwise constructs the inline Blob worker. This closes the source option
mapping without establishing the option used by the saved observations.
The projection does not identify which producer ran, so the inline slot-12
meaning is conditional on that binding. Neither width reveals UA text, a marker value,
HTTP identity agreement or a reason for the native refusal.

## Later own-JSDOM synthetic observation

The owner reports socket-denied result `db1eb359` with the same SDK bytes,
synthetic identity, URL and budget, but explicit own-JSDOM factory evaluation.
It has 90 fields/567 bytes and two PNG attempts, both delivered, none rejected.
The token was discarded and the child joined. These are supplied owner facts;
this source checkout performed no run and read no runtime capture.

Compared with prior synthetic `4a5cf5e6`, the owner found exactly two additions:
8/13 and 8/14, both `(5,2)`. Every other projected field/type/length is unchanged.
Against historical R14 browser `472150ef`, 8/13 remains different because its
browser shape is `(5,3)`, and 8/14 is synthetic-only. The owner reports 31
differing rows in that comparison. It is still not a paired R14 SDK capture.
No live inference, field values or causal/acceptance conclusion follows.

**8/14: new synthetic (5,2), absent in the two older observations.**
`r[gE]` (410593) fills a 10-by-10 image with random RGB values and fixed opaque
alpha, sums its RGB components, and writes it to a canvas. It assigns a load
handler to a bare `new Image` before setting its source to the canvas data URL.
On load it draws the image back, reads the pixels, sums their RGB components,
and returns `abs(sum_after - sum_before)`. The collector near 480050 sends
that result through the existing type-5 packed-integer path, including zero.

This statistic differs from 8/13's sum of individual absolute differences.
Opposing RGB changes can cancel, and alpha changes do not enter 8/14's sum.
Even an actual zero result would not prove identical pixels; `(5,2)` reveals
only the smaller packed width and does not establish zero. Missing prerequisites,
construction/readback failure or an unobserved load callback can omit the field.
The older browser's omission therefore identifies neither pixel behavior nor
the cause of any failed collection. The two delivered PNGs and new field
presence establish no browser-equivalent rendering or upstream acceptance.

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
Mappings through `6e751e6` are integrated on `browserless` as `24a846d`,
following the earlier `b93a425` integration. This completes the supplied
source mappings, not the shared browserless outcome. Native acceptance is
still unproven. Runtime qualification, requests and cleanup remain with
owner `544711e3-0b76-4cb4-9732-124a0486847e`.

Next source action: map a newly supplied structural difference or correct a
specific source finding. Record a dependency with its owner, required input
and last verified evidence. A completed lane or pending owner action does not
justify voluntarily blocking the shared goal. Keep its full objective and
accounting; do not clear, recreate or narrow it to close this source task.
The outcome remains a fresh valid mint and accepted native Framer inference,
with verified account isolation, matched identity/egress, cooldowns and joined
cleanup. Prompt guidance and actual app enforcement must be reported separately.

## Outstanding actions and evidence

The policy addendum reached all five participating threads. These were conversation
instructions; persisted objective strings, goal identities and accounting were
unchanged. The runtime lane acknowledged it in lifecycle-note commit `f3aa327`.
Transport/config owner: `mcp:a94287af-9994-4882-abc3-5108e653cc34`.

| Action | Owner | Required input | Last verified evidence |
| --- | --- | --- | --- |
| Qualify the next native inference comparison | Runtime owner named above | Owner-qualified package, fresh mint and retained account, egress and rate authority | R14 browser HTTP 200/output; isolated V2/native Castle 403; cleanup confirmed. `db1eb359` is socket-denied synthetic evidence only. |
| Extend the source mapping | This structure lane | Newly supplied structural projection or a specific source correction | All supplied differences mapped through `6e751e6`; owner reports all 16 unchanged comparator cases passing. |
| Enforce the requested goal-status policy in T3 | Transport/config owner | Verified enforcement change and accounting-preservation result | Guidance delivery is confirmed. Native enforcement remains unverified; do not use `/goal edit` while accounting preservation is unverified. |
