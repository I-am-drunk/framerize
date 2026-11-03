# Claude reliability audit

The audit starts from deployed alpha.25, source `36db08e2`.
Requested models, thinking effort, total output allowances and account ownership
must remain intact. Unknown completions must not trigger automatic replay.

## Findings

| ID | Status | Evidence and action |
| --- | --- | --- |
| C01 | Fixed in source, live verification pending | At 18:51:02 UTC, Opus 4.8 failed after 68,963 ms. Its adaptive request bypassed the Opus 5 checkpoint selector and allowed 64,000 upstream tokens. Add the exact Opus 4.8 IDs to the existing signed-checkpoint path. |
| C02 | Investigating | Other adaptive Claude models still use one buffered request. Qualify their history and continuation requirements before enabling checkpoints. |

C01's original-source regression fails because no continuation request occurs.
After correction, all 152 Messages tests pass. The new checks cover model IDs,
requested effort, tool definitions, signed content, unsigned-block rejection and
the prohibition on replay after an unknown completion.

These fixture results do not establish multi-day production reliability.
Live checkpoint verification and the remaining lifecycle audit are pending.
