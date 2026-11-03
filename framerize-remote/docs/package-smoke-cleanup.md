# Package smoke cleanup

Branch `fix/nexus-package-smoke-cleanup-20261103` makes package smoke remove
only its own newly allocated installation directory, whether the smoke passes
or fails. Supplied tarballs, existing directories, and npm's separate caches
are not cleanup targets. Package installation and runtime behavior are unchanged.

Validation on November 3, 2026, with Node.js 22.23.4:

- All 34 focused tests pass. They use real temporary files and module imports
  with mocked npm and CLI commands for Linux, macOS, and Windows code paths.
- Tests cover generated, relative, absolute, and downloaded tarballs; pack,
  install, help, npm exec, and import failures; missing or duplicate downloaded
  inputs; and CRLF checkout text. The original install-failure cleanup case
  fails against the pre-fix script.
- A real Linux installation of the retained `671ed7df` npm tarball passes
  with npm forced offline, private configuration and caches, and no inherited
  account configuration. The smoke's temporary installation is gone afterward.
  The input SHA-256 remains
  `12c6eb43755f16a858d12b150bfde44622298ab2fcf489d03abadba0bd4fe528`.

The first real-run wrapper expected the entire temporary parent to be empty;
npm's unrelated compile cache invalidated that assertion. The corrected check
scopes cleanup to smoke-owned directories and disables compile caching only in
the validation environment. It passes. No existing caches or evidence were deleted.

Next action: review this checkpoint and its actual three-platform CI before
merging. Mocked platform branches do not qualify native Windows or macOS runs.

## Offline fixture isolation

The follow-up branch `fix/nexus-package-smoke-isolation-20261103` gives npm
and installed CLI subprocesses a private home, cache, and npm configuration.
It does not inherit account paths, upstream credentials, npm authentication,
or Node preload settings. Packing and installation explicitly stay offline
and skip lifecycle scripts. Supplied tarball paths and cleanup are unchanged.

The previous script passed no subprocess environment and allowed npm install
to use the host's online configuration. A memory-only command probe confirmed
both properties without a registry request. The regression fixtures now check
the environment and npm flags with poisoned synthetic host configuration.

On November 3, 2026, all 35 focused tests pass on Linux with Node.js 22.23.4.
The pre-fix PR100 script fails the new isolation regression. A real offline
installation of the retained `671ed7df` tarball also passes with synthetic
account paths and credentials in the parent environment. Its SHA-256 remains
`12c6eb43755f16a858d12b150bfde44622298ab2fcf489d03abadba0bd4fe528`.
Only the unrelated sentinel remains in that run's temporary parent directory.
No pre-existing files or caches are removed.

Actual Windows and macOS execution of this follow-up remains a separate CI
gate. Earlier cleanup results do not prove this new isolation behavior.
