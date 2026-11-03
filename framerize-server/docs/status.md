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

The provider reentrant-close follow-up merged through PR #91. The source and
Nexus migration checkpoints are integrated at `7ddd669e`. Their existing evidence
does not qualify native server binaries.

## Native packaging checkpoint

Branch `feat/framerize-server-native-20261103` starts at `7ddd669e` and changes
only server packaging scripts, development dependencies, docs and server CI.
Runtime, provider and acceptance-owned files remain unchanged. Castle and all
other private runtime assets are excluded from the package and binary inputs.

The scripts build Node single executables, package canonical license notices,
and test extracted archives with isolated environment variables and loopback
fixtures. Final per-target manifests record the full clean source commit,
package version, Node version and hashes for five artifacts. Aggregation checks
all three platform manifests before writing the six shared downloadable payloads.

The workflow retains six source/npm jobs and adds one shared npm package job,
three native jobs and one aggregate manifest job. Native jobs target Linux x64,
macOS arm64 and Windows x64 on Node 22.23.4. They never publish or activate a service.

Node 22.23.4 passes the 84-test combined suite, 38-test acceptance entry point,
syntax checks and local packed-install smoke. Workflow YAML and job invariants
also pass a local parser check.

At clean commit `9a2486c9504339f8fdd180ce8013d06a7eb31e7f`, Linux x64 passes
`npm run check`, `npm pack --ignore-scripts`, the downloaded-tarball install
check, native build, archive packaging, extracted-binary smoke and final metadata
generation. The smoke verifies help, configuration, auth refusal, readiness,
both routes, exact SSE/gzip bytes, credential separation and graceful shutdown.
The five artifact hashes are retained in
`native-evidence/9a2486c9-linux-x64.json`. The binaries remain local ignored
build outputs, not published release artifacts. Build tooling emitted ELF
section-name warnings; the executable and archive checks passed.

Current-head platform and aggregate qualification remains an integration gate.
The first PR #97 run, `41658193948`, passed four source jobs but failed both
Windows package checks. npm removes CR from the installed launcher's shebang
while leaving its remaining CRLF bytes intact. A real local npm installation
reproduces the exact 275-versus-276-byte failure on the old checker. The fixed
checker accepts only that expected shebang conversion and passes the same
CRLF fixture plus the ordinary LF packed-install check. Native and aggregate
jobs were skipped in that run, not qualified.

Provider commit `8969774cb150d86626678cb9e0ce21a2a362e00f` is integrated unchanged
into this topic at `0848e05a0daf5b55f089fda3aeb7d79b7c2259c9`. The combined source
passes 86 tests, syntax, the shared-tarball install check and the full Linux x64
native build/package/extracted-smoke/metadata sequence on Node 22.23.4. Its five
new hashes are retained in `native-evidence/0848e05a-linux-x64.json`.
Earlier `9a2486c9` artifacts remain historical evidence, not qualification of
this provider update. The integration owner retains PR #96's merge decision.

An expanded binary smoke passes against the recorded `0848e05a` archive without
rebuilding it. The extracted executable remains healthy while a stream is open,
and client disconnection closes the corresponding upstream request before
server shutdown. This cancellation check is now part of each native CI job.
The smoke asserts that both sides are still open immediately before the client
disconnects. A negative control that ends upstream early fails that assertion.
Both checks pass using private temporary memory storage when local disk space
is insufficient for archive extraction.

All 11 jobs in [server CI run 41658627614](https://github.com/I-am-drunk/framerize/actions/runs/41658627614)
pass for PR head `3fff5b77`: six source/npm jobs, the shared package, all three
native targets and aggregation. The tested synthetic merge has the same tree
as that head. Exact manifests, artifact identity and qualification limits are
retained in `native-evidence/41658627614/`. Independent review verified all
payload and nested archive-member hashes. This checkpoint predates the added
disconnect assertions, so it is not a substitute for current-head CI.

At `d07fac36`, server CI run `41659675038` failed the macOS Node 24 readiness
queued-header fixture. Its immediate event-loop stall could prevent the test
response from flushing, so the test had not established queued input. The
provider owner's test-only correction
`40bf5c8788cdb5e06466db9f61701d21a0b4769a` is integrated unchanged as
`48dcb87c`. It waits for successful write completion before the stall. The
40 ms header deadline, 120 ms stall and provider implementation are unchanged.
Both corrected cases still fail against the pre-fairness provider in the
owner's Node 22 and 24 checks.

The integrated correction passes both focused header regressions, all 86 package
tests, all 38 acceptance tests and syntax checks on Node 22.23.4. Runtime,
launcher and workflow bytes are unchanged from `d07fac36`. The failed CI run's
skipped native and aggregate jobs provide no new qualification; the corrected
combined head still requires those jobs and the acceptance-owned packaging
regressions.

The acceptance owner's packaging checkpoint
`3021a3d089b4b010bf105ca53ad3840541b1cded` is integrated unchanged as
`4ba898a5`. Its five files add 18 regression tests and include them in the
acceptance entry point. They cover release identity, artifact and checksum
validation, canonical licenses, dirty checkouts, and exact npm-installed bytes
across launcher shebang conversion and downloaded-package line endings.
The synthetic build fixtures use the current host target without skipping
macOS or Windows. They do not build native binaries.

At this combined checkpoint, Node 22.23.4 passes syntax checks, all 106 default
test-discovery entries and all 56 acceptance tests. Default discovery includes
103 test cases and three helper-module entries. Node 24.13.0 passes all 18
packaging regressions. There are no failures, skips or cancellations. Runtime,
launcher, package metadata, build scripts and workflow bytes remain unchanged
from `89d1fcf6`. These local results do not replace fresh platform CI.

At `9e48ec02`, both Windows jobs in server CI run `41660697824` failed the
three tarball-mutation fixtures before their assertions. GNU tar interpreted
the absolute drive-letter archive path as a remote host. The acceptance
owner's correction `282005ab18398d81d3e84b5177a2f488f86b92e1` is integrated
unchanged as `6ce572ba`. Both tar operations now use a local archive basename
inside their temporary directory and copy the bytes in and out. All five
package-smoke tests pass locally on Node 22.23.4, including the three mutation
rejections. Independent review passes; actual Windows qualification remains open.

A prospective merge with integration base
`76ccf1fafde11d2ac6f23156882162a13b208a64` exposed a separate scope conflict.
That base adds parent-owned Castle exports, shipped assets and an installed
package smoke that constructs a minter. The authorized HTTP runtime files
remain unchanged, but the expanded npm package violates this stage's recorded
exclusions. The prospective tree was inspected without merging or executing it.
The parent owns resolving this package boundary before renewed combined CI.

Windows graceful POSIX-signal behavior is not claimed. Live inference,
private credentials, production activation and release publication remain
outside this stage.

Next action: resolve the integration package boundary, then qualify the portable
fixtures and final combined source/native/aggregate tree before integration.
