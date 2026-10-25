# R5 recovery descriptor result, 25 October 2026

The memory-open permission failure is corrected in the recovery controller.
The root helper opens only the plan-bound sidecar with read-only, close-on-exec
flags, rechecks its identity, transfers the descriptor, and closes its copy.
The controller checks the authenticated peer, process generation, payload,
descriptor target and access mode. Root executable inspection is avoided.

All 12 new descriptor tests and four affected lease tests passed without skips
in the existing isolated 128 MiB test envelope. Qualification collection:
`56d953d5215741e3e013c55bd7f817f2e0ee261ad9d13dcfbd615252798bbe18`.

Recovery r4 obtained the descriptor, established quiescence and restoration,
then refused the account-state comparison with `restored_primary_changed`.
Both held processes resumed. The ingress helper removed its restriction;
the helper processes, private socket and transient unit were absent at
collection. All source pins remained unchanged and the shared lock was free.
There were zero binary switches and zero inference requests in this attempt.

Failure collection: `940babbcaa66ca09b7f9a808dc0415794ea201430c47c9624720d6d00a72281c`.
The previous binary remains the recovery target. The primary-device mismatch
requires a bounded source and saved-state diagnosis; it is not permission to
discard the comparison or claim completed rollback or stream acceptance.
