# Status

Verified on November 3, 2026:

- 56 unit and integration tests pass on Node.js 24.
- GitHub CI test and package jobs pass on pull requests 47 and 48.
- The installed npm tarball runs outside the checkout.
- The isolated executable runs without Node.js on `PATH`.
- Local SSE streaming passes, including cancellation, backpressure, and
  fragmented UTF-8 and line endings.
- Graceful SIGINT and SIGTERM shutdown exit with code zero and close once.
- The release rehearsal passes from a fresh clone. It builds the executable,
  starts development and isolated servers, and verifies artifact checksums.
- Release metadata rejects mismatched version tags and source commits.
- Release `framer-next-v0.1.5` contains the verified executable archive,
  installable tarball, checksums, license, and source metadata.

The live connector listens on `127.0.0.1:42789` and uses 12.2 MB of RAM.
A live Opus probe through the connector still receives Framer's 403 security
refusal. The connector returns that failure as HTTP 503 instead of a successful
completion. Successful live Framer inference is not currently verified.

## Publication dependency

The `framer-next` package name remains available. First publication requires
an authenticated npm package owner. The registry returns `ENEEDAUTH`, so no
package has been published. After first publication, configure trusted
publishing for this repository, workflow, and `npm-release` environment.
