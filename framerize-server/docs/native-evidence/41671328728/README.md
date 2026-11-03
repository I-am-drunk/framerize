# Actual-main ENV server evidence

[Server CI run 41671328728](https://github.com/I-am-drunk/framerize/actions/runs/41671328728)
completed successfully with all 11 jobs. The automatic `push` event on `main`
built the actual PR #113 merge commit
`faf41ca24169e898dd02ca8224b8e07a3c1d90b4`.

Its tree `5f7db8a58d1a45f124e4f5e8b618bb9ea1e8360b` equals the qualified
synthetic merge `196189fcb13019189b909c22b3f0c73caa7bd7b5`. The server and
server workflow bytes equal PR head `c5f55245516beec727df57b8a2b6807eba839533`.
This is actual-main evidence, distinct from the earlier PR qualification.

## Artifact provenance

Artifact `13247733640`, named `framerize-server-qualified`, is 111341868 bytes.
Its GitHub ZIP SHA-256 is
`03133cb9482a9496a35d7106bb8d9ac691302b5bc32bda4ab3e0a580a2f795ae`.
GitHub reports creation at `2026-11-03T16:14:44Z` and expiry at
`2027-02-01T16:09:51Z`.

[release.json](release.json) and [SHA256SUMS](SHA256SUMS) preserve the previously
verified downloaded bytes without regeneration. No executable or archive is
committed here. The shared npm tarball has SHA-256
`da16a74b9f10df57d42c02b7a30d36587cea5446977b4ecfcb45b6cde619c44b`.
All three build records bind the actual main commit above and Node 22.23.4.

Core verification and independent review checked the ZIP digest and size,
all six payload hashes, nine nested executable/license hashes, exactly three
members per native archive, and all 13 npm files against the qualified Git
blobs. The project license matches source; both licenses are nonempty LF text.

## Qualified checks

All six source jobs passed 125 source tests, 69 acceptance tests and the
packed-install smoke, without skipped or cancelled tests or failure-diagnostic
output. Each job's log binds checkout `faf41ca2`. The three minimum-Node jobs
used 22.12.0; Linux used Node 24.22.2 and macOS/Windows used Node 24.22.1.

The shared npm job, three native jobs and aggregate job also passed. Native
logs bind the same checkout and Node 22.23.4 on Linux x64, macOS arm64 and
Windows x64. Downloaded-package and extracted-binary checks cover help,
configuration, auth, readiness, streams, gzip, disconnect cancellation and
cleanup. This retention reuses those completed checks and artifact verification;
it does not download, rebuild or rerun them.

## Verify retained bytes

The retained metadata SHA-256 values are:

- `release.json`: `481f33d8861c42387f38cbe7ce021adce1ca01229407ab6c5c81c9c16c327e8b`.
- `SHA256SUMS`: `aeea107b3334ae15d09bee7f62e9b3d8a9b3d9820eb3deda0164b7f3657f8888`.

These hashes describe the original LF bytes. Compare working-copy hashes only
in an LF-preserving checkout. Git's `core.autocrlf=true` can convert text to
CRLF. From the repository root, hash raw committed blobs to bypass that conversion:

```sh
git show HEAD:framerize-server/docs/native-evidence/41671328728/release.json | sha256sum
git show HEAD:framerize-server/docs/native-evidence/41671328728/SHA256SUMS | sha256sum
```

The retained `SHA256SUMS` lists six artifact payloads, not the metadata files.
Run `sha256sum -c SHA256SUMS` only alongside those unchanged downloaded payloads,
not in this metadata-only directory. Artifact expiry may prevent later retrieval.

## Limits and unresolved reset

The earlier Windows Node 24 `ECONNRESET` remains unexplained and instrumented.
Its original evidence is retained in
[run 41664532677](https://github.com/I-am-drunk/framerize/actions/runs/41664532677).
Passing checks do not establish its cause or a fix. The
[accepted disposition](https://github.com/I-am-drunk/framerize/pull/109#issuecomment-6608274989)
retains it as a known limitation after the required gates pass; any recurrence
blocks the affected checkpoint pending diagnosis and reviewed disposition.

This record qualifies fixture behavior at the exact source and artifacts above,
not later source, Nexus origin/probe/CLI integration, live inference, private
credentials, the separate library, or Windows POSIX-signal behavior. No release
publication, deployment or production activation is authorized or demonstrated.
