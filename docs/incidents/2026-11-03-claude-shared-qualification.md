# Shared Claude release qualification

The release source is `125fe257c9ef200823a560d24106fea2765b0eef`, version
`0.5.1-alpha.29`. Both packages contain the same request fixes. The bridge
reads checkpoint versions 1 and 2 and writes version 1. The default package
reads both versions and writes version 2.

The iMac built both Linux packages in the existing isolated build container.
The builds used one Cargo job, the locked offline dependency set and no
incremental compilation. The web source tree remains
`6a83e8aa815c13fabeb5858265df10d09aa91f71`.

Both archives passed independent manifest, executable identity, capability
and packaged lifecycle checks on Iris after the iMac's package checks.

| Package | Build seconds | Archive SHA-256 |
| --- | ---: | --- |
| Bridge | 1,088.454 | `57fa66e0f559c61dd776ef02d07d28fee801d4baa3eaa8ea8fcc1780055c05ac` |
| Candidate | 353.488 | `41a9d2bed4b348f2ab379b4b6fab24d8c8671134b64cc6401793dbd7f291e273` |

The bridge executable SHA-256 is
`7ff756f4af0467aadf9b7e3ba028532143c475eee32521bacb6bbb0325c941d3`.
The candidate executable SHA-256 is
`6c5ed7dabccb8fe3f09d97c13b0a7b39a182a7dd59a89d50928cc41fadd4b5dd`.

## Verification scope

Both exact packages passed archive and binary identity checks, their packaged
lifecycle checks, buffered and streaming OpenAI and Claude requests, signed
continuation and tool arguments. The same isolated account, device history
and checkpoint database continued through both packages. The final bridge
run accepted a signed conversation created by the version 2 writer.
No database snapshot was restored.

The last verified production release remains alpha.25, source `36db08e2`.
This qualification does not change the production service or its accounts.

## Bridge results

The bridge passed buffered and streaming OpenAI and Claude requests. Adaptive
Opus 4.8 completed the eight-section article through three checkpoints in
18.963 seconds and accepted the combined follow-up. Explicit-thinking Opus 5
produced three signed checkpoints and accepted its signed follow-up. Opus 4.8
published one complete 300-number tool call, preserved two signed blocks and
accepted the tool result.

The first attempt stopped on a fixture assumption: adaptive thinking can
produce zero thinking tokens. Captures showed that all upstream content
reached the client. The continuation reused the four passing protocol checks
and completed article from the identical executable, then verified the
remaining cases. Its article timing includes that gap and is not latency.

Shutdown took 3.280 seconds and left no owned processes. Peak cgroup memory
was 1,804,718,080 bytes. No memory or task limit events occurred. All 500 test
workspace credits remained, and existing checkpoint rows were unchanged.

## Candidate results

The candidate accepted the bridge's signed Opus 5 conversation after restart.
Buffered and streaming OpenAI and Opus 5.5 protocol checks passed. Adaptive
Opus 5.5 completed the eight-section article in 56.875 seconds through nine
checkpoints, preserved eight signed blocks and accepted the signed follow-up
in 1.731 seconds. The longest captured request-to-response interval for one
chunk was 6.539 seconds. Its requested 64,000-token allowance, xhigh effort, Fast
mode and adaptive thinking remained intact. Upstream and client content
matched exactly.

Explicit-thinking Opus 5 completed the article through three signed
checkpoints in 61.062 seconds and accepted the signed follow-up in 2.174
seconds. Opus 5.5 published exactly one complete 300-number tool call and
accepted its result. That adaptive tool reply contained no thinking blocks;
the separate explicit-thinking case verified signed history.
The streamed Opus 5 article crossed three checkpoints, preserved three signed
blocks and emitted exactly one successful terminator. Its combined follow-up
passed. The complete streaming case took 52.505 seconds.

Shutdown took 3.222 seconds and left no owned processes. Peak cgroup memory
was 1,639,575,552 bytes, with no memory or task limit events. All 500 credits
remained. The run added 12 checkpoint layouts, including version 2, and
preserved all 21 earlier layouts. The isolated account and workspace remained
outside production throughout verification.

## Return to the bridge

The bridge accepted the candidate's signed conversation in 16.046 seconds.
All 33 saved checkpoint layouts, including version 2, remained unchanged.
The test workspace retained all 500 credits.

Shutdown took 37.599 seconds and left no owned processes. This passed the
90-second cleanup limit but was slower than the other completed runs.
Peak cgroup memory was 1,704,828,928 bytes, with no memory or task limit events.
This run qualifies history compatibility and eventual clean shutdown; it
does not establish a three-second shutdown guarantee.

## Activation procedure

Production activation requires the user's approval after qualification.
The prepared procedure uses the existing release transaction and preserves
the installed service configuration, resource limits and account identities.
Both qualified packages are staged as immutable releases. A read-only live
preflight refused activation because production requests were active.
The service remains unchanged. Activation still requires approval and an
idle preflight at execution time.

1. Verify both exact packages and their recent isolated qualification receipts.
   Stage them as separate immutable releases.
2. Require the expected running process, unchanged service configuration,
   zero request leases, complete cleanup and no active security pause.
3. Activate the version 1 writer first. Confirm the expected executable,
   HTTP health, readiness and unchanged account identities.
4. Recheck the same conditions before activating the version 2 writer.
   Retain the bridge as its rollback package.
5. On startup or readiness failure, use the existing transaction to restore
   the preceding compatible executable while preserving the entire database.

Alpha.25 cannot read version 2 history. Preparation confirmed that direct
activation of the default package from alpha.25 fails before service
shutdown. Once version 2 rows exist, alpha.25 is no longer an acceptable
rollback.

## Evidence limits

The [Claude audit](2026-11-03-claude-reliability.md) records the source tests
and all 16 findings. The existing GitHub build and CodeRabbit checks were
skipped. They do not count as passing verification or an external review.
Eight isolated tests pass for the prepared activation guard, including
changed process, configuration, account and resource state, active requests,
and unconfirmed cleanup. These tests perform no service operations.

Framer's upstream gateway can still return HTTP 504 after about 60 seconds.
A larger local timeout cannot extend that deadline. A caller's explicit
thinking budget can require a chunk larger than the normal checkpoint limit.
Unknown completion still prohibits automatic replay.

SSE error events preserve the provider protocol, but a client SDK decides
whether to retry after receiving response headers. The qualification does
not establish uninterrupted multi-day operation or strict provider-side
enforcement of Anthropic beta headers.
