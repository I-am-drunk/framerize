# Server core status

The core implements the API version 1 contract in the committed server plan.
Implementation files are inside `framerize-server/`; the separately authorized
`.github/workflows/server-ci.yml` qualifies this package. The provider and
acceptance owners maintain their respective source and tests; core integration
preserves their committed files.

## Evidence

- At `b46eae2775f1909429c5baf11164873cbc0371ed`, Node 22.23.4 and 24.13.0
  combined suites pass 82 tests: 38 acceptance tests, 43 provider tests, and
  the fixture helper import. No tests fail, skip, or cancel.
- The acceptance entry point passes 38 tests on Node 22.23.4. Five focused
  regressions also pass: real-provider error redaction, queued response bytes
  after an event-loop stall, paused-consumer recovery, close during listen,
  and reentrant close.
- Syntax checks pass for all source, CLI, and scripts on Node 22.23.4.
- The packed npm tarball installs offline outside the checkout on Node 22.23.4.
  Its CLI help, health, authenticated readiness, and inference fixture pass there.
- Focused checks pass for CLI SIGTERM, private-file permissions, FIFO rejection,
  metadata size limits, circular-log redaction, startup shutdown races, and
  connection-nominated request headers.

The acceptance owner's commit `bb8c39123a44aef0222fb9e8d22f397a37df60ac` is
integrated unchanged as `b46eae2775f1909429c5baf11164873cbc0371ed`. It adds
durable coverage for the reviewed transport and lifecycle fixes, metadata
rejection and freezing, silent and cyclic logging, missing-token denial,
Connection header filtering, and upstream-error redaction.

PR #89 merged into `framer-next` as
`266a055e43b81b7e8caad68439509fb95d39fc99`. Its server files and dedicated CI
workflow match the tested PR head `6bebaa26931edaf957bff069f79a14c0edf2c1ab`.
[Post-merge server CI run 41653732588](https://github.com/I-am-drunk/framerize/actions/runs/41653732588)
passed all six jobs at that merge commit: Linux x64, Windows x64, and macOS arm64,
each on Node 22.12.0 and 24. Every job passed architecture verification,
`npm ci`, syntax checks, tests, acceptance, and the installed-package smoke check.

## Ownership and handoff

Core-owned files in this change:

```text
framerize-server/LICENSE
framerize-server/README.md
framerize-server/package.json
framerize-server/package-lock.json
framerize-server/bin/framerize-server.js
framerize-server/examples/metadata.example.json
framerize-server/docs/status.md
framerize-server/scripts/check.js
framerize-server/scripts/package-smoke.js
framerize-server/src/accounts.js
framerize-server/src/cli.js
framerize-server/src/config.js
framerize-server/src/lifecycle.js
framerize-server/src/logger.js
framerize-server/src/server.js
framerize-server/src/transport.js
.github/workflows/server-ci.yml
```

The provider owner supplied `src/provider.js`, `src/errors.js`, and
`test/provider.test.js`. The acceptance owner supplied `scripts/acceptance.js`,
`test/accounts.test.js`, `test/config.test.js`, `test/logger.test.js`,
`test/routes.test.js`, `test/shutdown.test.js`, `test/streaming.test.js`,
`test/integration.test.js`, and `test/helpers/fixtures.js`, all beneath
`framerize-server/`.

PR #89 merged the core and acceptance fixtures. PR #88 merged the provider.
The integration owner retains control over later activation and release.
Dedicated CI runs fixtures and packed-install checks
on Linux x64, Windows x64, and macOS arm64 using Node 22.12.0 and 24. Native binary
qualification belongs to the server release stage, independently of Nexus binaries.
No live inference, deployment, or release publication has been performed.

Provider commit `6a928f3b` fixes the paused-consumer idle-timeout failure found by
Windows CI. The core uses the same one-poll expiry deferral. Focused provider and
real-provider-through-server regressions pass locally, including genuine idle and
header stalls. The real-provider integration fixture preserves queued bytes
after a 100 ms event-loop stall with a 40 ms idle timeout, without increasing
the timeout or adding a total-duration deadline.

Next action: review the provider's reentrant-close follow-up and support the
separate Nexus package qualification. Native server binaries and live activation
remain outside this source and npm-package qualification.
