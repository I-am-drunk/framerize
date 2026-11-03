# Post-merge native CI evidence

[Server CI run 41664738426](https://github.com/I-am-drunk/framerize/actions/runs/41664738426)
completed successfully with all 11 jobs. The `push` event on `framer-next`
built the actual PR #97 merge commit
`d3621ff36b96465ef63552752098dbc32205284a`.

Its tree `d3c6254cf11ee3892728505a9f49c89fc578a7d0` equals qualified PR head
`23d87c6c9a04952514eed9eebc6936e61b6f3e05`. This run is distinct from the
earlier PR qualification retained in `../41663296507/`.

Artifact `13246276309`, named `framerize-server-qualified`, is 111339616 bytes.
Its GitHub ZIP SHA-256 is
`2ce68f2666522bd0e172fe6b45dd48b7f27f5d77e84c087cca92c69e9b5dbc96`.
GitHub reports creation at `2026-11-03T15:35:31Z` and expiry at
`2027-02-01T15:31:19Z`. The retained `release.json` and `SHA256SUMS`
match the downloaded artifact byte-for-byte. No executable is committed here.

Core verification checked the ZIP digest and size, all six payload hashes,
all nine nested executable/license hashes, exactly three members per native
archive, and all 13 npm files against the qualified Git blobs. Each build
records the actual merge commit. The project license matches source, and
both licenses are nonempty LF text. The shared npm tarball retains SHA-256
`c4282b29d43ee8fa38856cd26a4778db82926548fb329d753e5306924d2c2ff3`.

Six source/npm jobs cover Linux x64, Windows x64 and macOS arm64 on Node
22.12.0 and 24. The shared npm package job, three native jobs using Node
22.23.4, and aggregate manifest job also passed. Artifact verification did
not rebuild or rerun the executables.

This passing run does not explain or dismiss PR #109's separate Windows
Node 24 failure in
[run 41664532677](https://github.com/I-am-drunk/framerize/actions/runs/41664532677).
Its real-provider error-redaction test received `ECONNRESET`. That failure
remains under investigation; no runtime correction is claimed by this record.

This is post-merge fixture qualification at the recorded commit, not evidence
for a later source tree or a merge to `main`. It does not qualify live
inference, private credentials, the separate library, Windows POSIX-signal
behavior, publication or production activation.
