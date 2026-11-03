# Native CI evidence

[Server CI run 41658627614](https://github.com/I-am-drunk/framerize/actions/runs/41658627614)
completed successfully with all 11 jobs passing. The event was `pull_request`
at head `3fff5b77cdc0af36eda6407c4a55fddb9f23ad2d`.

The build used GitHub's synthetic merge
`ef3303f8ff4a9a3134b7de4ae2f3122c9dce1e55`, with parents
`7ddd669e9cad1812f8aa7d6848317f79dcd3f37a` and the PR head above.
Its tree `3394c535bdc163640be7e3e178f1b2f521bef3ce` exactly matches the PR head.

Artifact `13240169186`, named `framerize-server-qualified`, contains 111339613
bytes. Its GitHub ZIP SHA-256 is
`f73cb2de3773ebad1d5ac41301a1efbf1152b153bf2756a1ecb0f85ad74de874`.
GitHub reports creation at `2026-11-03T14:58:56Z` and expiry at
`2027-02-01T14:56:12Z`. The retained `release.json` and `SHA256SUMS` match
the downloaded artifact byte-for-byte. No executable is committed here.

Independent review checked the ZIP digest, all six aggregate payload hashes,
and all nine nested executable/license hashes. Each platform archive has exactly
one executable, `LICENSE` and `NODE-LICENSE`. All three builds use Node 22.23.4.

The workflow tested six source/npm combinations, one shared npm tarball,
three extracted native executables and one aggregate manifest. This covers Linux
x64, Windows x64 and macOS arm64. It does not qualify later source revisions,
later disconnect-smoke assertions, live inference, publication or activation.
