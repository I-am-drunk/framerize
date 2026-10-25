# Recovery primary-selection measurement, 25 October 2026

The recovery-only diagnostic ran once against the unchanged rollback
generation. It made no binary switch and no inference request. Its bound
plan was `1f18d3a5dd6e2dcb7e1bfedb2dc107aefa48dcba1507faa7f4dcaffc4a34fbc8`.
Eight new diagnostic/provenance checks passed with no skips; sixteen unchanged
memory/lease checks were retained. No accepted test was rerun for publication.

The account comparison measured eleven retained lifecycle records. Four
primary selections changed from null to a device, all from resident accounts
without a pending restoration marker. One resident device selection, one
resident null selection, and five restored null selections were unchanged.
The aggregate checks found no missing active cooldowns or request-window
entries, reduced mint spacing, or reduced same-day reserve consumption.

The existing strict primary-equality predicate still raised
`restored_primary_changed`. No transfer clear or finalization ran. Both service
processes resumed; the helper removed its temporary ingress restriction.
All known helpers, the transient unit and private socket were absent, source
bindings were unchanged, and the shared execution lock was free at collection.
Three bound client sockets were interrupted during this authorized measurement.

This localizes the comparison failure but does not prove which past request
selected each device. The frozen host permits ordinary sticky selection after
restoring the saved primary. A finite source correction is assigned for only
null-to-known-device transitions with exact resident/no-marker provenance.
All other transitions and all limit/identity checks remain outside that exception.
The correction is not qualified or deployed. Rollback finalization and the
repair's stream acceptance remain incomplete.

Aggregate evidence SHA256:
`05aab93073ca905adbebd163b6d7722ce173f45184d090e4e7ab901cb90d57b9`.
Controller terminal 66832 exited 1; authentication terminal 6344 exited 0.
