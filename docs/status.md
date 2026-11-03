# Status

Verified on November 3, 2026:

- 18 unit and integration tests pass on Node.js 24.
- GitHub CI test and package jobs pass on pull request 32.
- The installed npm tarball runs outside the checkout.
- The isolated executable runs without Node.js on `PATH`.
- Live local SSE streaming passes.
- One real Opus Responses request returned HTTP 200 and completed normally.

The live connector listens on `127.0.0.1:42789` and uses the existing local
Framer endpoint at `127.0.0.1:8787/openai`.

## Publication dependency

The `framer-next` package name is available, but this machine has no
authenticated npm user. The registry returns `ENEEDAUTH`, so no package has
been published. Publication remains pending until npm authentication or
trusted publishing is configured.
