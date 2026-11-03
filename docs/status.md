# Status

Verified on November 3, 2026:

- 56 unit and integration tests pass on Node.js 24.
- GitHub CI test and package jobs pass on pull request 40.
- The installed npm tarball runs outside the checkout.
- The isolated executable runs without Node.js on `PATH`.
- Local SSE streaming passes, including cancellation, backpressure, and
  fragmented UTF-8 and line endings.
- Graceful SIGINT and SIGTERM shutdown exit with code zero and close once.
- The release rehearsal passes from a fresh clone. It builds the executable,
  starts development and isolated servers, and verifies artifact checksums.
- Release metadata rejects mismatched version tags and source commits.

The live connector listens on `127.0.0.1:42789` and uses 12.2 MB of RAM.
A live Opus probe through the connector still receives Framer's 403 security
refusal. The connector returns that failure as HTTP 503 instead of a successful
completion. Successful live Framer inference is not currently verified.

## Publication dependency

The `framer-next` package name is available, but this machine has no
authenticated npm user. The registry returns `ENEEDAUTH`, so no package has
been published. Publication requires npm authentication or the protected
`npm-release` environment and npm trusted publishing for this repository.
