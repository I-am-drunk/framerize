# Claude reliability audit

The audit starts from deployed alpha.25, source `36db08e2`.
Requested models, thinking effort, total output allowances and account ownership
must remain intact. Unknown completions must not trigger automatic replay.

## Findings

| ID | Status | Evidence and action |
| --- | --- | --- |
| C01 | Fixed in source, live verification pending | At 18:51:02 UTC, Opus 4.8 failed after 68,963 ms. Its adaptive request bypassed the Opus 5 checkpoint selector and allowed 64,000 upstream tokens. Add the exact Opus 4.8 IDs to the existing signed-checkpoint path. |
| C02 | Investigating | Post-activation logs contain 21 buffered failures for Opus 5.5, which also bypasses checkpoints. Qualify its stricter signed-history requirements before enabling them. |
| C03 | Immediate capacity restored | Iris exhausted disk space. Four terminated log records contain partial writes at page boundaries, and the journal records `No space left on device`. The user approved deleting 29 incremental compiler cache directories older than 24 hours. Revalidating their paths, inodes, timestamps and recursive sizes reclaimed 2,957,383,974 logical bytes. Source, binaries, account state and logs were excluded. |
| C04 | Fixed in source, deployment pending | Rust's event writer retained its first daily file until restart, and retention ran only at startup. The writer now rotates on the first event of a new UTC day, reapplies retention and retries failed opens. Retention preserves the active file, symlinks and unrelated files. All nine logging tests pass. |
| C05 | Fixed in source, deployment pending | Full layouts at every checkpoint caused quadratic growth. New rows refer to their saved parent and retain only the new turn. All earlier rows remain readable. Twelve history tests and 160 Messages tests pass, including restart, legacy-layout compatibility, corrupt links and full-storage failures. |
| C06 | Fixed in source, deployment pending | An empty final chunk tried to replace the already-published content's layout, turning completion into a history error. Empty chunks now retain the last valid layout. The regression failed before the fix and now passes. |

C01's original-source regression fails because no continuation request occurs.
After correction, all 152 Messages tests pass. The new checks cover model IDs,
requested effort, tool definitions, signed content, unsigned-block rejection and
the prohibition on replay after an unknown completion.

These fixture results do not establish multi-day production reliability.
Live checkpoint verification and the remaining lifecycle audit are pending.

## Checkpoint storage limits

At 32 and 64 chunks, the original layout data occupied 20,171 and 76,123 bytes.
The same fixture now uses 5,971 and 12,211 bytes. These are SQLite row payloads,
not complete file sizes. Every published prefix remains restorable.
The 128 MiB database cap remains. A full disk still stops new publication
instead of deleting records needed by old signed conversations.

The new linked layout is version 2 and reads existing version 1 rows without
rewriting them. An older binary cannot decode new linked rows. Production
activation must preserve the history database and retain a rollback binary
that supports both versions. Restoring an old database snapshot would lose
post-activation conversations and is not an acceptable automatic rollback.
