# Historical status

This record predates Framerize Remote. It preserves the original package names
and evidence. Use the [current guide](../README.md) for commands and status.

The package migration to Framerize Nexus merged through
[PR90](https://github.com/I-am-drunk/framerize/pull/90). The package lives in
`framerize-nexus/`. The package, CLI, executable, and new release tags
use `framerize-nexus`. Existing Framer Next release names below are historical.
No Nexus package has been published or activated. The next contract change
follows server native-artifact qualification and the separately reviewed
acceptance gates. Publication remains disabled pending owner approval and a
verified publication path.

Current Nexus evidence on Node.js 22.23.4 and 24.13.0:

- Package installation and offline `npm exec` pass outside the checkout.
- The extracted Linux x64 executable passes help, configuration, and loopback
  server smoke tests with no Node executable on `PATH`.
- Release fixtures cover platform artifacts, source identity, checksums,
  missing artifacts, branch rehearsals, and publication guards.
- [PR90 CI run 41654625616](https://github.com/I-am-drunk/framerize/actions/runs/41654625616)
  passed all six jobs at commit `671ed7df4507b391fc6efc72bf919aebd289a1eb`, including
  downloaded-package and extracted-binary smokes on Linux x64, macOS arm64,
  and Windows x64.
- [Release rehearsal 41654672886](https://github.com/I-am-drunk/framerize/actions/runs/41654672886)
  passed all five jobs for that exact commit through an explicit branch
  `workflow_dispatch`. All platforms ran Node.js 22.23.4. Linux and macOS
  passed 113 tests each. Windows passed 111 tests with two pre-existing POSIX
  signal tests skipped in `test/cli.test.js`.
  Assembly verified the shared tarball, platform manifests, and all six
  downloadable payload hashes. An independent download verified all six
  payloads against the aggregate `SHA256SUMS`.

The rehearsed package version is `0.1.9`. Its tarball SHA-256 is
`12c6eb43755f16a858d12b150bfde44622298ab2fcf489d03abadba0bd4fe528`.
The `release-evidence-41654672886` artifact retains `release.json`, which binds
the source commit, platform executable and archive hashes, licenses, and Node
versions. These results apply to the recorded commit; later test and
documentation changes do not represent another release rehearsal.

The original manifest and checksums are also retained in
[the committed rehearsal record](release-evidence/0.1.9-671ed7df/README.md).
The GitHub artifact expires on February 1, 2027; preserving its binary payloads
requires a separate owner-controlled archive before expiry.

Historical Framer Next evidence, verified on November 3, 2026:

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

The historical check found the `framer-next` package name available and npm
authentication missing. These results do not establish availability or
ownership of `framerize-nexus`.

Publication dependency: package owner, npm authentication. Last verified:
November 3, 2026, with a private token request timing out and `npm whoami`
returning `ENEEDAUTH`. This historical handoff does not authorize registry
checks or publication for Nexus. Its package ownership, publishing npm/OIDC
support, and enforceable approval path remain separate owner decisions.

The `framer-next-v0.1.7` GitHub release contains its verified tarball,
executable archive, checksums, Node license, and source metadata. The release
build passed. Its npm step failed with `ENEEDAUTH` after the GitHub release
was published.

## Historical Framer Next handoff

Owner: Framer Next maintainers. Required input: authenticated npm package-owner
credentials. Last verified evidence: November 3, 2026, `ENEEDAUTH` after a
private credential request timed out. This handoff does not authorize Nexus
publication. Nexus requires explicit owner approval of the version, source
SHA, and artifact hashes.
The owner-authenticated GitHub token has administration access. GitHub rejects
required reviewers under the private repository's current billing plan.
