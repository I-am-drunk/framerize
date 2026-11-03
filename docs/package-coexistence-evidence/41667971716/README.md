# Hosted package-pair qualification

[Run 41667971716](https://github.com/I-am-drunk/framerize/actions/runs/41667971716)
completed successfully on 2026-11-03 for the `pull_request` event. Its head was
`1187aad19b2275738b85d2ede17f98ef096c9049` on `ci/package-coexistence-20261103`.
The producer and all three consumer jobs passed.

All four raw checkout logs identify `3a4e79b409ed1911ea53da311113ace9a63b7408`.
That merge commit has parents `4b942b480a584f920a4f6247750f8407d9c3d556`
and `1187aad19b2275738b85d2ede17f98ef096c9049`. Its tree is
`ebe751ce7e0b0c90e2697e198400fe748520cbfd`, matching the reviewed composition.

Linux x64, macOS arm64 and Windows x64 each passed all 41 root tests, without
skips, on Node 22.23.4 and npm 10.9.9. Each consumer installed the same two
producer tarballs offline in a private directory. Installed bytes, distinct
CLI targets, help/check commands and baseline module exports passed the checker.
This qualifies this installed package pair, not native binaries or later changes.

## Original reports

The seven JSON files preserve the downloaded artifact bytes without rewriting:

- [Producer manifest](manifest.json).
- Linux x64: [report](linux-x64-report.json), [checker](linux-x64-checker.json).
- macOS arm64: [report](macos-arm64-report.json), [checker](macos-arm64-checker.json).
- Windows x64: [report](windows-x64-report.json), [checker](windows-x64-checker.json).

All three reports bind producer manifest SHA-256
`f99c7c3012bf8b655b3abfdf1eadd64d8acdf6b1e0fcca79595c18626ebcfc9b`.
[SHA256SUMS](SHA256SUMS) records every retained JSON digest. Verify these text
files from this directory with `sha256sum -c SHA256SUMS`.

The workflow ran at attempt 1. This does not claim an actual failed-only rerun;
attempt 1 producer reuse by an attempt 2 consumer has separate fixture evidence.

## Exact package inputs

- `framerize-nexus-0.1.9.tgz`, 11988 bytes, SHA-256
  `12c6eb43755f16a858d12b150bfde44622298ab2fcf489d03abadba0bd4fe528`.
- `framerize-server-0.1.0.tgz`, 15531 bytes, SHA-256
  `c4282b29d43ee8fa38856cd26a4778db82926548fb329d753e5306924d2c2ff3`.

Each package contains 13 files. A separate byte comparison matched every
packed file to the Git blobs at the exact checkout above. All four artifact
ZIP sizes and SHA-256 digests matched GitHub's recorded artifact metadata.
Every consumer report and checker output matched the producer package hashes.

These baseline packages predate the new server ENV and thin-Nexus changes.
Neither later runtime bytes nor the separate Castle library are qualified here.
The checker imports two Nexus and five baseline server module exports. The import
probe does not invoke providers; CLI checks initialize and close configuration.
These checks do not make live requests, authorize publication or activate services.

## Artifact retention

GitHub reports the same expiry for all four artifacts: `2027-02-01T15:50:13Z`.
The original uploaded ZIP identities are:

- Producer `13246753146`, `package-coexistence-41667971716-packages`, SHA-256
  `bf766502e65bb9f5653011a34d055dc9abc403ad7a49824989adf6799864ac64`.
- Linux `13246524180`, `package-coexistence-41667971716-report-linux-x64`, SHA-256
  `2efc8471596c033ad98034227848a03305d6bba9d364d5ce2084c1b6c8ea1f0d`.
- macOS `13246233416`, `package-coexistence-41667971716-report-macos-arm64`, SHA-256
  `7d8704a5077a9669be945df8d89954a016ef6b8e8114957a62649b33a910fe33`.
- Windows `13246054284`, `package-coexistence-41667971716-report-windows-x64`, SHA-256
  `64cc0150cd9eaeec475e21b1c8f6d46caf4684b6fb118ba9305608fe4bdf9029`.

Only text evidence is committed. The tarballs, artifact ZIPs and raw logs remain
outside Git. An owner must archive those separately before expiry if future
byte recovery is required. Preserving these reports does not rebuild artifacts,
extend GitHub retention, or qualify a later source commit.
