# Final native CI evidence

[Server CI run 41663296507](https://github.com/I-am-drunk/framerize/actions/runs/41663296507)
completed successfully with all 11 jobs. The `pull_request` event used head
`23d87c6c9a04952514eed9eebc6936e61b6f3e05`.

The build checkout was synthetic merge
`3bc02ea7a89018764a1f81df05cc8cf752503c20`, with parents
`1e13ade1d95a1212f68d72c0a640c31f3214e267` and the PR head.
Its tree `d3c6254cf11ee3892728505a9f49c89fc578a7d0` equals the PR head tree.

Artifact `13244516128`, named `framerize-server-qualified`, is 111339599 bytes.
Its GitHub ZIP SHA-256 is
`31b43cc24612f2aa407df4cb1ed6fccd74cf03dca68fa2c7a28df18dd749e1ca`.
GitHub reports creation at `2026-11-03T15:26:52Z` and expiry at
`2027-02-01T15:22:57Z`. The retained `release.json` and `SHA256SUMS` match
the downloaded artifact byte-for-byte. No executable is committed here.

Core verification and independent review checked the ZIP digest and size,
all six payload hashes, all nine nested executable/license hashes, exactly
three members per native archive, and all 13 npm files against frozen Git
blobs. The project license matches source; both licenses are nonempty LF text.

Six source/npm jobs cover Linux x64, Windows x64 and macOS arm64 on Node
22.12.0 and 24. Both Windows versions pass 117 test entries, 67 acceptance
tests and packed-install smoke. All three native builds use Node 22.23.4.
Their extracted-binary checks pass help, configuration, auth, readiness,
streams, gzip, client-disconnect cancellation and cleanup.

This is PR fixture qualification, not post-merge or later-source evidence.
It does not qualify live inference, private credentials, the separate library,
Windows POSIX-signal behavior, publication or production activation.
