# Connecting collector results to final inputs

Pinned source: Castle SDK SHA256
`3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71`.
At byte 282619, `Q[_o]=l(s,Qon(W))` selects position 242. At byte
284204, `Q[473]=a(X0,krn(W))` selects position 473. The surrounding
`l(s,...)` and `a(X0,...)` dispatches return their argument unchanged.

`apply_collector_result` connects the existing native SharedWorker and
network reducers to those positions in a supplied value array. It does
not grow the array, manufacture missing observations or certify readiness.
Reduction failure leaves the entire array unchanged. All other positions
remain the caller's responsibility.

Eight comparisons execute the exact original assignment statements against
765-position arrays. Three tests cover matching assignments, unchanged
arrays on refusal and rejecting an absent target position without growth.
The underlying reducer comparisons remain valid and were not repeated.

A second check records the two actual inputs in one isolated synthetic
SDK operation. It captures primitive values and array data properties at
the two callsites, returns the original values, and retains callback order.
This fixture probe is not a general serializer for arbitrary page objects.
The native check clears both positions, reapplies the recorded outcomes
in observed order (473 then 242), and compares the complete serialized
array and final encoding with that same SDK operation.

That check passes: 765 positions, 5,412 serialized bytes and a matching
5,297-byte final output by SHA-256 and length. The 128 MiB SDK sandbox's
denial and cleanup checks pass; the native check uses the 512 MiB compiler
harness. No account or upstream request is used. Other collector behavior,
genuine device observations, live acceptance and memory savings remain open.

## Completed network-hash assignment

The field47 trace adds `Q[_C]=l(k4,Brn(W))` at byte 284185, with `_C=469`.
`Brn` at 429659..429849 selects the original hash or a missing-field marker;
the dispatch applies `Cr` at 207029..207266. This UTF-16 transform is already
used by the native envelope timestamp, so both consumers now share it.

`CollectorResult::NetworkHash` applies this one assignment before Network.
The ordinary completed N_ record supplies a string. Missing rows retain
their error, and falsey fields select the original marker. Pre-normalized
JSON and truthy non-string hashes are explicitly refused. The latter need
JavaScript length/charCodeAt behavior outside this data-only boundary.
Accessor side effects and replaced helpers are not modeled or claimed.

The new comparison passes 101 original-helper cases, including five
missing-row errors, plus normalized-input and missing-target preservation.
Affected address, assignment and envelope checks pass without behavior changes.
The retained synthetic capture now rebuilds positions 469, 473 and 242 in
that order. Complete serialization and final output still match the same
5,412-byte / 5,297-byte observation. No new SDK operation or live request ran.

## Complete network callback

The remaining source input is bound by HANDOFF SHA256
`ae7f25c47a447edbe35e0c9980af0ac616347dc5b585122b3c178a3d501f9a3c`
and binding SHA256
`e3b615fe153f71203d73b7ee9ffe31bbcf896fc41c1e6113dc328f3f804d10a3`.
At 284106..284245, the callback writes related address Q[80], hash Q[469],
address Q[473], then protocol summary Q[659], after its separate timing write.

`network_related_address` implements T0n/id/Cr: field40, at 554645..554825,
95918..95986 and 207029..207266. `network_protocols` implements a$/i1/Cr:
field18, at 367910..368101, 1267..1304 and the same Cr range. Both preserve
the original falsey-field and rejected-outer markers before transformation.
Missing rows remain errors. Truthy non-string fields are explicitly refused.
These APIs model plain data; a$'s repeated getter reads and replaced helper
bindings are not represented by the native value model.

Apply NetworkRelatedAddress, NetworkHash, Network and NetworkProtocols in
that order, stopping on the first failure. Each assignment leaves its own
destination unchanged on error. Earlier writes remain; later slots retain
their previous values. No rollback or false readiness is implied.

The new metadata comparison passes 202 original-helper cases and 13 full
callback cases, including failures after earlier writes. The existing hash,
address and assignment checks also pass: 23 tests in total, zero skipped.
The retained SDK capture passes a further test after rebuilding all five
positions (80,469,473,659,242) and the complete unavailable network record.
Its 5,412-byte serialization and 5,297-byte final output still match. No new
SDK operation, live account, service action or RAM-saving claim follows.

## Single network assignment entry point

`apply_network_result` now consumes one observation and lazily transforms
fields 40, 47, 38 and 18 into positions 80, 469, 473 and 659. It moves fields
out of the supplied record instead of cloning it. Each destination is checked
before its conversion; failure preserves earlier assignments and stops later
ones. Individual `apply_collector_result` calls remain available.

The changed code passes 25 focused tests, including 14 assignment-sequence
comparisons, an object record and refusal to extend incomplete destinations.
A further saved-snapshot test reproduces the full serialized and encoded
output using the new entry point. Timing and live collection remain separate.
