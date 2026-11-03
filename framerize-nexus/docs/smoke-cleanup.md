# Binary smoke cleanup

The `fix/nexus-smoke-cleanup-20261103` follow-up removes only the temporary
directory allocated by the current binary smoke invocation. Cleanup covers
archive extraction, license validation, CLI help, configuration checks, and
the loopback server fixture. It waits for child closure with a five-second
bound, closes fixture connections, and retries removal for transient locks.
Child cleanup timeouts still fail the smoke test.

Validation on November 3, 2026:

- Node.js 22.23.4 passes all 31 focused cleanup tests. Linux, macOS, and Windows
  branches use mocked processes and extraction commands, not native OS runs.
- An explicit CRLF-loaded script fixture verifies Windows checkout parsing.
  This changes only the test input, not package or runtime bytes.
- The pre-fix script fails the new partial-extraction cleanup assertion.
- One actual Linux x64 smoke passes help, configuration, and loopback streaming
  using the retained `671ed7df4507b391fc6efc72bf919aebd289a1eb` release archive.
  Its SHA-256 is
  `0477c41ec959b41575a6d10d2d8cfe44e032714c45d217789b0e35943b4116ec`.
  The newly allocated temporary parent is empty afterward, and the archive
  digest is unchanged.

Existing temporary directories and historical evidence remain untouched.
No package runtime, server files, release workflow, or activation changes are
part of this fix. Current-head macOS and Windows CI remain required; the Linux
result does not replace them. Next action: review the cleanup checkpoint and
its three-platform CI before merging.
