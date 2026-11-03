# Status

The package migration to Framerize Nexus is in progress on
`t3/rename-connector-framerize-nexus`. The package now lives in
`framerize-nexus/`. The package, CLI, executable, and new release tags
use `framerize-nexus`. Existing Framer Next release names below are historical.
No Nexus package has been published or activated. Next action: finish local
fixture and packaging verification, then review the migration before choosing
its first version tag.

Verified on November 3, 2026:

- 91 unit and integration tests pass on Node.js 24, with 100% line coverage.
- Pull requests 75 through 78 merged after their GitHub test and package jobs
  passed.
- The publication helper has 100% line, branch, and function coverage. It
  verifies source, artifact, registry, and authentication preconditions.
- The official 0.1.7 release tarball and executable archive pass their
  published checksums and run outside the checkout.
- The installed npm tarball runs outside the checkout.
- The isolated executable runs without Node.js on `PATH`.
- The Framer fixture verifies request envelopes, session identifiers, short
  responses, and incremental Chat Completions streaming.
- Local SSE streaming passes, including cancellation, backpressure, and
  fragmented UTF-8 and line endings.
- Graceful SIGINT and SIGTERM shutdown exit with code zero and close once.
- The release rehearsal passes from a fresh clone. It builds the executable,
  starts development and isolated servers, and verifies artifact checksums.
- Release metadata rejects mismatched version tags and source commits.
- Release `framer-next-v0.1.5` contains the verified executable archive,
  installable tarball, checksums, license, and source metadata.

The live connector listens on `127.0.0.1:42789` through an enabled persistent
`systemd --user` service. Release 0.1.7 is activated with a retained rollback
binary. See `docs/activation.md` for its source commit and artifact hash.
A live direct Framer probe through the isolated Node connector returned HTTP
200 with `response.status` set to `completed` and the text `OK`. It used the
existing private account session and fresh local credential service, without
copying secrets into JSON or logs. The model was `gpt-6-sol`. Live Opus support
is not established by this probe.

The isolated native Chat probe returned HTTP 200 with the exact text `OK`.
It used the original Chat Completions payload and the existing private account
credentials. The binary shut down cleanly afterward.

The activated connector also passed two live Opus Chat streams using
`anthropic/claude-opus-5.5`. The short stream returned `OK` and `[DONE]`. The
long stream delivered its first chunk in 3 ms, all 240 chunks, 40 numbered
lines, and `[DONE]` in 14.4 seconds. The service remained active at about
20 MB afterward. No live inference errors were logged.

A live GPT-4.1 Chat request returned HTTP 200 with `finish_reason` set to
`tool_calls` and the requested `weather` call for Paris. The equivalent Opus
request returned Framer's explicit `paid_plan_only_model` response. This is an
upstream workspace restriction, not a connector or Castle failure.
A streamed GPT-4.1 Chat request also returned the complete `weather` call for
Paris and its terminal `[DONE]` marker.

Version 0.1.7 adds native Chat tool-schema and streamed tool-call regression
tests. Routine client disconnects now produce informational logs. Its package,
standalone executable, short and long stream fixtures, and live streamed tool
call have passed verification.

After activation, a short Opus stream returned HTTP 403. The workspace still
enforces its `paid_plan_only_model` restriction. This is an upstream account
entitlement, not a streaming or tool-transport failure. The earlier successful
Opus short and long streams remain recorded above.

## Historical Framer Next publication dependency

The `framer-next` package name remains available. First publication requires
an authenticated npm package owner. The registry returns `ENEEDAUTH`, so no
package has been published. After first publication, configure trusted
publishing for this repository, workflow, and `npm-release` environment.

Publication dependency: package owner, npm authentication. Last verified:
November 3, 2026, with a private token request timing out and `npm whoami`
returning `ENEEDAUTH`. Do not repeat authentication checks until the owner
provides credentials. Run `npm run publish:package` for the verified release tarball,
then verify registry installation and configure trusted publishing.

The `framer-next-v0.1.7` GitHub release contains its verified tarball,
executable archive, checksums, Node license, and source metadata. The release
build passed. Its npm step failed with `ENEEDAUTH` after the GitHub release
was published.

## Active handoff

Owner: Framer Next maintainers. Required input: authenticated npm package-owner
credentials. Last verified evidence: November 3, 2026, `ENEEDAUTH` after a
private credential request timed out. Next authorized action: publish the
verified release tarball, test its registry installation, then configure
trusted publishing. Goal blocking is disabled by user policy. No further local
verification or authentication polling is needed before credentials arrive.
After authentication, run `npm run publish:package`. It rechecks authentication,
rejects an existing version, verifies the packed artifact, publishes the
package, and chooses its dist-tag.
The owner-authenticated GitHub token has administration access. GitHub rejects
required reviewers under the private repository's current billing plan.
