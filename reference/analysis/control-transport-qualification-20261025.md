# Normal control-transport qualification

The changes from `a715a41a` are now integrated into this experimental branch.
Normal control clients have a ten-second connection deadline and a thirty-second
total deadline. Transport failures preserve safe typed cause flags without
retaining raw error text; the existing HTTP status policy is unchanged.

All ten selected tests passed, with zero selected skips. Their source declares
29 scenarios; this is not a separate count of registered tests. The test root
includes the real complete `transport.rs`, `classify.rs` and `error_body.rs`
files and the unchanged transport fixtures. Native-only feature methods and
unrelated API fixtures were excluded. This qualifies normal control transport,
not the complete native feature graph or a deployed application.

The focused test build finished in 12.545 seconds, peaking at 143,421,440 bytes.
The 768 MiB compiler limit was unchanged. The test execution peaked at
21,938,176 bytes. Both runs retained the shared lock, private network, input
masks, host reserves, zero swap and original time limits. Input and output
hashes were verified, known processes and cgroups were absent after collection,
and no memory or task limit event occurred.

The first preparation omitted a static fixture include. The second reached
the compiler memory cap while compiling unrelated API tests. Both failures
remain recorded in `control-transport-compile-gap-20261025.md`; neither is a
passing result or a demonstrated production transport defect.

Aggregate result SHA256:
`5a21b31c6640446ad8274d782ea336c9d70444c61b9030aa0c3c4b5b421910f7`.
Compilation terminal 89913 and execution terminal 73175 both completed with
exit zero and were collected separately. No live request or service action
ran as part of this qualification. Existing repair and V3 packages are unchanged.
