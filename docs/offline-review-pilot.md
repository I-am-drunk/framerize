# Offline review packet pilot

This is the first local component for the proposed PR-Agent integration.
It neither fetches GitHub data nor runs PR-Agent, a model or a subprocess.
It has no review publisher. It cannot approve, merge, change repository settings
or activate a binary. Generated results say `runtime_verified: false` and
`authority: none`, including after successful contract validation.

The candidate image is the researched PR-Agent 0.49.0 CLI digest recorded in
`review-system.md`. No image was installed or its attestation verified here.
The generated settings have not been tested through PR-Agent or Framerize.
They are proposed settings, not evidence that the requested wire fields work.

## Prepare a packet

`python3 -I -B scripts/review_packet.py prepare snapshot.json policy.json
--head FULL_HEAD_SHA --base FULL_BASE_SHA --output /private/packet.json`

The snapshot is normalized local JSON with exactly `pull` and `files`.
`pull` contains `repository`, `number`, `state`, `head`, `base`, `changed_files`,
`additions` and `deletions`. The canonical repository is `I-am-drunk/framerize`.
Each file has `path`, nullable `previous_path`, `status`, `additions`, `deletions`
and a complete GitHub-style `patch`. Supported statuses are added, removed,
modified and renamed. Binary, missing, unsupported or truncated patches refuse
preparation. Pure renames may have an empty patch and have no line anchors.
Counts must agree across patches, files and the pull request. Both supplied
commit pins must match, and no duplicate file path is accepted.

Unchanged spans before and between hunks must have equal old/new lengths.
Empty ranges use the unified-diff convention of naming the preceding line.
Added files must have one complete new-content hunk starting at line 1, an
empty old side, and only added lines. Removed files require the reverse.
Neither status accepts context lines or changes on the nonexistent side.

An end-of-file marker must immediately follow a content line and cannot repeat.
It ends the old side after a deletion, the new side after an addition, or both
sides after context. No remaining hunk content, later content or skipped lines
may extend an ended side. Changes on the other side can still follow, including
in a later hunk with an empty range at the ended side's exact position.

The policy contains exactly `model`, `effort`, `max_input_tokens` and
`max_output_tokens`. The model is `gpt-6-astra-ultrafast`; effort and positive
integer limits are required, with no default. Output limits below 4096 are
refused because the pinned PR-Agent 0.49.0 handler would raise them to 4096.
Accepted limits remain unchanged. This minimum follows
[`MIN_MAX_OUTPUT_TOKENS` and `_configured_max_output_tokens()` in the pinned source](https://github.com/The-PR-Agent/pr-agent/blob/efdde4afe14aa401a3889012c7f3e9ef3081a430/pr_agent/algo/ai_handlers/litellm_ai_handler.py).
It is a compatibility constraint, not a chosen default or a usage budget.
Limits here are proposed model
configuration, not observed token usage or a tokenizer qualification.
The generated PR-Agent settings disable fallback models, repository settings,
publication, model routing, callbacks and parallel suggestion calls. No endpoint
or credential is stored. Text inside a patch never supplies configuration.

The CLI reads at most 2 MiB per regular input file and uses exclusive mode-0600
output creation. On POSIX, it opens without blocking and refuses FIFO input.
Compact output uses the same 2 MiB limit as input. It does not replace an existing file. Errors never print input JSON contents.

## Validate a proposed result

`python3 -I -B scripts/review_packet.py validate packet.json result.json
--packet-sha256 SAVED_PACKET_HASH --output /private/validation.json`

The caller must retain the expected packet hash independently. A hash supplied
alongside untrusted data is not authentication. The packet hash covers the
snapshot, full patches, selected policy, candidate image and generated settings.
The normalized snapshots themselves are local inputs, not verified API receipts.

The result contains exactly `packet_sha256`, `model`, `effort`, `completed`,
`covered_files` and `findings`. The declared model/effort must match; completion
must be true; the coverage list must name every file exactly once. These are
worker claims, not independently observed model execution or analysis coverage.
Missing coverage, changed packets and incomplete results refuse validation.

Each finding contains `path`, `side`, `line`, `severity`, `problem`, `evidence`
and `correction`. Only changed LEFT/RIGHT lines and P1/P2/P3 severities are
accepted. Text is bounded, control characters are refused, and exact duplicate
findings are refused. Stable fingerprints include the packet identity. Output
is JSON only. A future publisher must escape it and apply its own rules before
rendering any Markdown or HTML. This component does not defeat model prompt
injection or establish that a finding is correct.

## Remaining integration

A trusted collector must authenticate the requesting actor, fetch all PR pages,
normalize these records, and check head/base before and after collection and
analysis. Never treat `author_association` or a local snapshot as permission.
An isolated worker still needs image provenance verification, real PR-Agent
fake-provider tests, deadlines and joined cleanup. Verify model, effort, token
limits, route, fallback behavior and reported coverage before live inference.
A separate publisher and quality evaluation remain future work. No Actions
workflow or service consumes these files yet.

The 16 focused tests exercise the actual CLI, stale/partial data, settings
isolation, coverage, line anchors, duplicates, private exclusive outputs,
nonblocking FIFO refusal, hunk continuity, added/removed semantics and the
pinned output minimum. EOF regressions cover marker placement, ended-side
content and skipped ranges, plus valid changes on the other side. All tests
use synthetic inputs and perform no network I/O.
