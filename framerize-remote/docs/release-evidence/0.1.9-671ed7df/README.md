# Nexus 0.1.9 rehearsal evidence

These are unchanged copies of the verified aggregate `release.json` and
`SHA256SUMS` from the historical, evidence-only release rehearsal.

- Source commit: `671ed7df4507b391fc6efc72bf919aebd289a1eb`
- Workflow run: [41654672886](https://github.com/I-am-drunk/framerize/actions/runs/41654672886)
- Event: `workflow_dispatch`
- Branch: `t3/rename-connector-framerize-nexus`
- Package version: `0.1.9`
- Build runtime: Node.js `v22.23.4` on all three platforms
- Artifact: `release-evidence-41654672886`, ID `13237694976`
- Artifact ZIP SHA-256: `73d31774561cae3114d4e7872c83d9bb5f4074280abb3824dc3a2a6ddc23058f`
- GitHub artifact expiry: `2027-02-01T14:32:24Z`

All five rehearsal jobs passed. Linux and macOS passed 113 tests each;
Windows passed 111 with two pre-existing POSIX signal skips. Each platform
installed the same npm tarball and ran its extracted native archive outside
the checkout. Aggregate verification covers the three archives, npm tarball,
and both licenses. The manifest also records each native executable hash.

The later migration head `4cfa6e09dec891332f59bf55237e75c81232e756` has identical
runtime, launcher, package metadata/lock, packaging scripts, and workflow
bytes. Only documentation and tests changed after the recorded source.
The manifest intentionally retains its original source commit.

This record does not authorize publication or activation and does not qualify
later server or library work. No binaries are stored here. An owner who needs
binary recovery must archive the original artifact before expiry. Text hashes
alone cannot recover expired payloads. Verify the downloaded ZIP against its
digest above and the extracted payloads with `sha256sum -c SHA256SUMS`.
