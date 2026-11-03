# Server core status

The core implements the API version 1 contract in the committed server plan.
Implementation files are inside `framerize-server/`; the separately authorized
`.github/workflows/server-ci.yml` qualifies this package. The provider and
acceptance owners maintain their respective source and tests; core integration
preserves their committed files.

## Evidence

For the later actual-main ENV checkpoint, see the
[retained run 41671328728 evidence](native-evidence/41671328728/README.md).

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

`npm run check` now validates the approved package files, exports, executable,
regular, optional and peer runtime dependencies, and source inventory before test
execution. It reads metadata and directory names only, without importing runtime
modules. Twelve isolated positive and negative cases pass on Node 22.23.4 and
24.13.0, including rejection of the expanded base's metadata without executing
its source. The acceptance owner maintains the durable guard regressions.

The acceptance owner's guard checkpoint
`8cd14bd807ebc7afa68c5f860aaaf879f8c47a7a` is integrated unchanged as
`3e910368`. Its four tests cover the approved inventory, all three runtime
dependency categories, extra source and executable files, and inert expanded
package metadata. The integrated guard tests and syntax checks pass on
Node 22.23.4. The owner's exact combined archive passes all 107 explicit tests
on Node 22.23.4 and the four guard tests on Node 24.13.0.

Parent restoration checkpoint `648d7299a664166b2d924b40cf1edab92357d7ba`
merges cleanly with core `afb3f373` in a read-only prospective merge.
The resulting tree `5a02a481b6641b78382d9c2161048fb168671170` changes only
package manifest formatting within the server and its dedicated workflow;
the parsed manifest is identical. The server inventory excludes the separate
library. This static check does not execute or qualify that library, merge
the restoration, or replace final combined platform and native CI.

The corrected restoration `f150471477cee48a303cd75b3b495dd3b3f6e060` and
core `aeae6595` produce prospective tree
`c752f419736b294dd18eed284101c1c44a97cd43`. A server-only archive of this
tree passes `npm run check` and the offline installed-package loopback smoke
on Node 22.23.4. Runtime, scripts, tests, lockfile and server workflow match
the current core exactly. No separate-library files were extracted or run.

The installed-package smoke also rejects regular, optional and peer runtime
dependencies. An offline mutation fixture showed that the previous check
accepted downloaded tarballs with optional or optional peer dependencies.
The unchanged package still passes, and both altered manifests now fail with
the expected dependency error on Node 22.23.4 and 24.13.0. These checks do not
install the synthetic missing dependencies or call an external service.
The smoke validates npm invocation before creating its temporary directory.
Direct invocation without npm now fails without leaving files behind, verified
on Node 22.23.4 and 24.13.0 against the old leaking behavior.

Installed-package verification also compares the complete manifest after npm's
leading `./` executable-path normalization, checks the complete installed
inventory, and requires regular files before importing the application.
The unchanged package passes; altered exports, executable aliases, package-file
lists and an extra inert source file all fail on Node 22.23.4 and 24.13.0.
The five existing package smoke tests and the real server's packed-install
check pass on Node 22.23.4. The reviewer reproduced acceptance of all four
substitutions on the earlier verifier; no current artifact compromise is claimed.

PR #102 restored the server package boundary in integration commit
`1e13ade1d95a1212f68d72c0a640c31f3214e267`. Core merged that base without
rewriting existing checkpoints as `1d0aca8d725409debea22a7240b4e5c5af4e4a72`.
The server's only incoming change is semantically identical manifest formatting.
Its runtime, scripts, tests, lockfile and workflow retain the reviewed bytes.
The merged tree passes syntax and offline installed-package checks on
Node 22.23.4. The separate library remains outside server execution and qualification.

Acceptance checkpoint `ce1762b755c74147b174db90f6244d872679c2da` is integrated
unchanged as `5fc517d4`. It retains seven regression cases for installed
dependencies, metadata, extra files and cleanup after invalid invocation.
The restored combined tree passes syntax checks, all 117 default test entries
and all 67 acceptance tests on Node 22.23.4, with no failures, skips or
cancellations. Default discovery includes 114 cases and three helper entries.
The owner also verified all 12 package smoke cases on Node 24.13.0; the real
merged server's installed-package smoke passes on Node 22.23.4 and 24.13.0.

Server CI now accepts matching pushes to both `framer-next` and `main`.
Its existing path filters, read-only permissions, dedicated package directory
and source/native/aggregate jobs are unchanged. YAML checks cover that policy;
actual post-merge qualification still requires a run at the merged source.

## Final PR qualification

At frozen head `23d87c6c9a04952514eed9eebc6936e61b6f3e05`,
[server CI run 41663296507](https://github.com/I-am-drunk/framerize/actions/runs/41663296507)
passes all 11 jobs. Both Windows source jobs pass all 117 test entries,
67 acceptance tests and packed-install smoke. All three native targets pass
extracted-binary checks, including the client-disconnect preconditions.
This closes the earlier Windows fixture and final aggregate gates.

Core verification and independent review pass the downloaded aggregate's ZIP
digest and size, all six payload hashes, nine nested executable/license hashes,
and exact npm inventory and file bytes. The synthetic merge tree equals the
frozen head tree. Exact provenance, manifests, hashes and qualification limits
are retained in `native-evidence/41663296507/` without executable files.
This docs-only retention does not change the qualified runtime or packaging.

Windows graceful POSIX-signal behavior is not claimed. Live inference,
private credentials, production activation and release publication remain
outside this stage.

## Post-merge qualification

PR #97 merged as `d3621ff36b96465ef63552752098dbc32205284a`. Its entire tree
equals the qualified PR head. Post-merge
[server CI run 41664738426](https://github.com/I-am-drunk/framerize/actions/runs/41664738426)
passes all 11 jobs. Core verification also passes the new aggregate's ZIP
digest and size, all six payload hashes, nine nested executable/license hashes,
and all 13 npm files against the qualified Git blobs. Exact post-merge manifests
and artifact provenance are retained in `native-evidence/41664738426/`.

PR #109's separate Windows Node 24 failure remains open. In
[run 41664532677](https://github.com/I-am-drunk/framerize/actions/runs/41664532677),
the real-provider error-redaction test receives `ECONNRESET`; 116 of 117
source test entries pass. A native incoming-header timeout can produce a
408 followed by a reset, but this mechanism does not establish the CI cause.
No deadline increase or speculative runtime correction has been applied.
The passing post-merge run does not close this investigation.

Acceptance checkpoint `600888610a5c2f68f8dcc632fa39a236d2e84c7d` is integrated
unchanged as `7c7613a1`. It adds failure-only, monotonic event diagnostics to
the affected test without changing deadlines, assertions or native event
handling. Both integration tests pass on Node 22.23.4 and 24.13.0. A deliberate
assertion-failure control emits the expected safe events on both versions
without fixture credentials or upstream-secret text. The owner's exact combined
archive passes 114 explicit tests and 67 acceptance tests on Node 22.23.4.
These results validate the diagnostics, not a fix for the Windows reset.

Next action: inspect the instrumented CI evidence, resolve the Windows finding,
and complete parent-controlled documentation integration and `main`
synchronization. Publication and activation remain excluded.
