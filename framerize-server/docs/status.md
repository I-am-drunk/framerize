# Server core status

The core implements the API version 1 contract in the committed server plan.
Implementation files are inside `framerize-server/`; the separately authorized
`.github/workflows/server-ci.yml` qualifies this package. The provider and
acceptance owners maintain their respective source and tests; core integration
preserves their committed files.

## Evidence

- Node 22.23.4 and 24.13.0 combined suites pass 74 tests, including the fixture helper import.
- The acceptance entry point passes 30 tests.
- Syntax checks pass for all source, CLI, and scripts.
- The packed npm tarball installs offline outside the checkout. Its CLI help,
  health, authenticated readiness, and inference fixture pass there.
- Focused checks pass for CLI SIGTERM, private-file permissions, FIFO rejection,
  metadata size limits, circular-log redaction, startup shutdown races, and
  connection-nominated request headers.

## Remaining integration work

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
`test/routes.test.js`, `test/shutdown.test.js`, `test/streaming.test.js`, and
`test/helpers/fixtures.js`, all beneath `framerize-server/`.

PR #89 carries the core and acceptance fixtures. PR #88 carries the provider.
The integration owner reviews and merges these changes and retains control over
later activation and release. Dedicated CI runs fixtures and packed-install checks
on Linux x64, Windows x64, and macOS arm64 using Node 22.12.0 and 24. Native binary
qualification belongs to the server release stage, independently of Nexus binaries.
No live inference, deployment, or release publication has been performed.

Provider commit `6a928f3b` fixes the paused-consumer idle-timeout failure found by
Windows CI. The core uses the same one-poll expiry deferral. Focused provider and
real-provider-through-server regressions pass locally, including genuine idle and
header stalls. Updated Windows CI remains a separate integration gate.

Next action: integrate the additional acceptance regressions, record their focused
results, and hand the tested PR to the integration owner.
