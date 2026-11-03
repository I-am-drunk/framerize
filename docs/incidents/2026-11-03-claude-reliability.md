# Claude reliability audit

The audit starts from deployed alpha.25, source `36db08e2`.
Requested models, thinking effort, total output allowances and account ownership
must remain intact. Unknown completions must not trigger automatic replay.

PR #125 now replaces model-specific Messages branches with shared continuation,
validation, history and error handling for every Claude model. Both alpha.29
packages passed isolated live qualification. The
[current package evidence](2026-11-03-claude-shared-qualification.md) records
their exact source and executable identities. Production remains alpha.25.
The alpha.26 results below and the alpha.27 build are historical evidence.

## Findings

| ID | Status | Evidence and action |
| --- | --- | --- |
| C01 | Alpha.29 verified in isolation, deployment pending | At 18:51:02 UTC, Opus 4.8 failed after 68,963 ms. Its adaptive request bypassed the Opus 5 checkpoint selector. The alpha.29 bridge completed an eight-section article through three checkpoints in 18.963 seconds and accepted its follow-up. The 64,000-token allowance, xhigh effort and Fast mode remained intact. |
| C02 | Alpha.29 verified in isolation, deployment pending | Logs contained 21 buffered failures for Opus 5.5. The prior model allowlist excluded it and other Claude models. The shared path removes that allowlist. The candidate completed the Opus 5.5 article through nine checkpoints in 56.875 seconds, preserved eight signed blocks and accepted the signed follow-up. Live coverage samples Opus 4.8, Opus 5 and Opus 5.5; fixtures cover the shared model-independent path. |
| C03 | Immediate capacity restored | Iris exhausted disk space. Four terminated log records contain partial writes at page boundaries, and the journal records `No space left on device`. The user approved deleting 29 incremental compiler cache directories older than 24 hours. Revalidating their paths, inodes, timestamps and recursive sizes reclaimed 2,957,383,974 logical bytes. Source, binaries, account state and logs were excluded. |
| C04 | Fixed in source, deployment pending | Rust's event writer retained its first daily file until restart, and retention ran only at startup. The writer now rotates on the first event of a new UTC day, reapplies retention and retries failed opens. Retention preserves the active file, symlinks and unrelated files. All nine logging tests pass. |
| C05 | Fixed in source, deployment pending | Full layouts at every checkpoint caused quadratic growth. New rows refer to their saved parent and retain only the new turn. All earlier rows remain readable. Regressions cover restart, legacy layouts, corrupt links and full-storage failures. The bridge deliberately retains the old writer until the compatible reader is deployed. |
| C06 | Fixed in source, deployment pending | An empty final chunk tried to replace the already-published content's layout, turning completion into a history error. Empty chunks now retain the last valid layout. The regression failed before the fix and now passes. |
| C07 | Regression verified, deployment pending | Truncated tool recovery could retain later thinking after removing the tool block it depended on. Recovery now retains only the exact prefix before the first unpublished tool. Its regression failed before the fix. Incomplete calls remain unpublished and unexecuted. Both exact packages also passed complete 300-number tool round trips. |
| C08 | Both packages and return path verified, deployment pending | Activation ignored checkpoint formats. Packages now obtain readable and written formats from the executable. Activation checks both readers and existing rows before stopping the service. The bridge accepted a signed conversation created by the version 2 writer and preserved all 33 saved layouts. Direct alpha.25-to-candidate activation was rejected before shutdown. Rollback never restores an old database snapshot. |
| C09 | Fixed in source | Native Anthropic conversion discarded provider extensions and appended text to the system prompt. It now preserves those fields. Request validation returns HTTP 400 before browser work. |
| C10 | Fixed in source | HTTP and SSE errors disagreed on retryability, and some provider statuses became generic errors. Permanent failures now retain their status and use the Anthropic error type in both forms. SSE failures after content prohibit replay and never emit a success terminator. |
| C11 | Fixed in source | Temporary security refusals could terminate a stream or repeatedly enter admission without a shared budget. The selected account now records each refusal once. Admission and in-chain cooldowns share a 960-second wait budget, with cancellation and stream pings. Permanent or unclassified provider refusals do not enter this wait. |
| C12 | Fixed in source | Messages output did not participate in shared output admission. The first correction still released capacity while a client retained emitted bytes. A regression reproduced that release. Reservations now survive the producer, body and all retained buffer clones. |
| C13 | Fixed in source | A closed event channel could generate pings without waiting. Rendering now waits for the producer or the next heartbeat. Terminal drain also processes content queued after noncontent events. |
| C14 | Fixed in source | Output queue overflow silently dropped content, and replacing its error could conceal unconfirmed browser cleanup. Overflow now stops further generation, reports an unreplayable failure and preserves cleanup uncertainty. |
| C15 | Fixed in source | A local Responses output-capacity refusal lacked a completion receipt and falsely marked application cleanup unconfirmed. The shared-capacity regression reproduced this when a Messages response occupied the only slot. Capacity and producer-admission refusals now complete their local ownership without claiming any upstream work occurred. |
| C16 | Existing usage semantics documented | Generated output tokens sum across checkpoints. Input tokens retain the largest upstream context count, while provider extensions such as cost, cache details and thinking-token details come from the final chunk. A real three-chunk response confirmed this behavior. Those extension fields are not a cumulative request bill. The isolated workspace credit balance is verified separately. |

The current 106 protocol, 495 API, 502 server and 15 native refusal tests pass.
The final shared continuation constructor passes all 165 Messages tests.
The legacy writer passes 13 history tests, including version 2 reads.
All 39 package, activation, release and build-identity tests pass.
The normal production application compiles without test-support features.
Both exact alpha.29 packages passed isolated verification and history
compatibility checks. The live preflight refused activation while production
requests were active. Earlier test counts below apply to their recorded
revisions, and are not additional current-source coverage.

## Historical qualification

C01's original-source regression failed because no continuation request occurred.
The alpha.26 correction passed 152 Messages tests at that revision.

Alpha.26 also completed a signed tool round trip and buffered and streamed
protocol checks. A manual Opus 5.5 chain preserved seven separate upstream
turns and completed its signed follow-up. This checks provider acceptance,
not the alpha.27 automatic checkpoint implementation. Strict provider-side
beta-header enforcement was not tested.
The isolated service exited normally in 3.012 seconds with no owned processes
left. Rate and checkpoint histories remain intact. All 500 credits remained.
Its cgroup recorded no memory-limit, out-of-memory or task-limit events.

These results do not establish multi-day production reliability.
Current-package results are recorded separately in the alpha.29 evidence.
Uninterrupted multi-day operation remains unverified.

## Remaining service limits

A caller's explicit thinking budget can require a chunk larger than the normal
1,200-token ceiling. The gateway preserves that budget. Such a request can
still hit the upstream deadline; a local 180-second timeout cannot extend a
gateway that returns HTTP 504 at 60 seconds. Unknown completion remains final.

The server emits standard SSE errors, but client SDKs decide whether to retry
after headers have arrived. A retryability field cannot force that behavior.
Strict beta-header enforcement and uninterrupted multi-day operation remain
unverified. Output admission bounds concurrent responses; it is not a hard
cap on process or browser memory.

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

The bridge build uses the same request fixes, reads both history formats and
continues writing version 1. Activate and verify it before enabling version 2
writes, then retain it as the candidate's rollback. Alpha.25 ceases to be a
valid rollback once any version 2 history has been written.
