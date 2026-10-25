# V3 network record field 38

Source: SDK SHA256
`3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71`,
Git blob `0b59745864e8045f650e08dd0ed53056defeac95`. Offsets are bytes,
zero-based with the end excluded. This is a static source result.

`N_` (354334..360014) resolves a record whose field `[0][38]` is the first
element of its local pair. The ordinary paths supply an empty string, a
timeout marker `"-"`, or a selected dotted-decimal candidate address.
Unsupported setup and offer rejection use the empty pair. The timer uses
the timeout pair. Candidate selection can replace it with the address pair.
Other callbacks may finalize the current pair without changing that value.

The address guard (3900..3951) checks a primitive string and a dotted-decimal
pattern. It is not complete IP validation or evidence of the route used.
Under standard intrinsics and stable RTCIceCandidate properties, this field
is a string, not the candidate object or accumulated connection metrics.
The guard and assignment read the property separately, so an arbitrary
changing getter is outside that conclusion.

`ud` (95795..95918) selects field 38 with an empty-string fallback. `krn`
(428835..429015) keeps a truthy selected value and substitutes an encoded
marker for an empty value. Its result reaches `Q[473]` at 284081..284260.
Thus an empty record field does not imply an empty serialized field.

The producer trace closes the ordinary field's type question. It does not
implement the network collector or establish callback ordering in execution.
The native `network_address` now implements that reduction for supplied plain
data with the built-in `ud` getter present. The original has guards for an
undefined, null or non-function getter; replacing that JavaScript binding is
outside this native API. In the original ordinary path, a missing first
record throws on property access. A missing field in an existing record
instead selects `2nrh3qm`. Non-object outer input selects `a9opjk`.

All 57 differential cases pass against the original `krn`, `ud` and serializer,
including five original TypeErrors. Nine additional checks refuse normalized
JSON at the outer, record and selected-value boundaries. The comparison uses
synthetic values only; no network activity, full SDK or live account was used.
