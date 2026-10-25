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
