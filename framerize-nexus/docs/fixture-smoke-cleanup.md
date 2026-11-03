# Fixture smoke cleanup

Branch `fix/nexus-fixture-cleanup-20261103` closes the remaining temporary
resource gaps in the development and Framer-envelope smoke scripts. Their
fixture requests and assertions are unchanged. Both original scripts passed
while leaving their account-fixture directory behind in an isolated run.

Cleanup now covers startup, account-fixture writes, configuration loading,
and request failures. The development smoke waits for child closure with a
five-second bound. Both scripts close fixture servers before deleting only
their newly allocated directory. The envelope smoke also restores the two
temporary credential environment variables to their prior values.

On November 3, 2026, all 14 focused cases pass on Linux with Node.js 22.23.4.
The tests launch actual subprocesses and use only loopback fixture endpoints.
A test-only preload injects write, listen, malformed-configuration, fetch,
spawn, early-exit, and readiness-timeout failures. Before removal it checks
that children and listening servers have closed. Every case preserves an
unrelated sentinel file and verifies restoration of synthetic environment values.

The preload is inactive unless explicitly selected by the fixture environment.
Syntax checks and `git diff --check` pass. No package runtime source, other
package, workflow, existing evidence, or pre-existing temporary directory is
changed. There are no live inference, publication, or activation requests.

The regular test jobs cover Ubuntu and Windows. The original macOS platform
job runs package and binary smokes, which do not exercise these two scripts.
A separate automation follow-up runs `test/smoke-cleanup.test.js` in that
existing macOS job before its binary build. No extra native build is added.

Next action: verify the focused macOS step on the automation checkpoint.
Earlier green six-job runs do not establish macOS fixture-cleanup coverage.
The Linux subprocess results do not establish other operating-system results.
