# Package coexistence CI acceptance

This gate qualifies the installed Nexus and server npm packages together.
The workflow is under implementation. The requirements below are not a claim
that macOS or Windows qualification has already passed.

## Scope

Use the existing [root coexistence checker](package-coexistence.md), from the
repository root, without a root npm workspace or installed build dependencies.
Do not change either package's runtime or its existing CI workflow.

The gate uses Node 22 on Linux x64, macOS arm64, and Windows x64. It does not
build native executables, exercise the separate Castle library, make live
inference requests, publish packages, or activate a service.

## One producer, three consumers

1. Pack Nexus and server once each on Linux with offline mode and lifecycle
   scripts disabled. Use private npm configuration and cache directories.
2. Retain those exact two tarballs with a producer manifest containing their
   names, versions, filenames, SHA-256 hashes, checkout SHA, and Node/npm versions.
3. Download that producer artifact on every OS. Verify the checkout identity,
   package identities, and actual tarball hashes before installing anything.
4. Run the root regression tests and the checker against the downloaded pair.
   Compare its package names, versions, filenames, and hashes with the producer.
5. Retain each consumer report with its checkout SHA, OS, architecture, and
   Node/npm versions. Do not substitute newly packed tarballs on consumers.

Fixture subprocesses must not inherit account paths, upstream credentials,
proxy settings, or Node preload options. The existing checker supplies private
installation directories and rejects network calls and listeners. Preserve
that isolation and its cleanup behavior.

## Automation requirements

The workflow runs for relevant pull requests, pushes to `main` and
`framer-next`, and manual dispatch. Path filters include both package trees,
the root checker, its tests, this workflow, and its focused regression tests.
Permissions stay at `contents: read`; do not use `pull_request_target`.

Missing artifact uploads fail explicitly. Artifact names must remain valid
for branch names containing slashes and support reruns without name collisions.
Consumer evidence must refer to the producer bytes it actually tested,
including when only failed jobs are rerun.

Focused tests must exercise the validation code, including wrong source SHA,
missing, duplicate or unexpected package evidence, altered tarball bytes, and
a checker report whose identity or hashes disagree with the producer. Workflow
string matching alone cannot prove these failure paths.

## Evidence required to close this gate

Retain the workflow run and event, exact checkout SHA, producer manifest, all
three successful consumer reports, and the uploaded artifact identities.
Check all reports against the same producer hashes. A green Nexus or server
native-build workflow does not qualify this root package-pair gate.

The earlier retained pair passed on Linux Node 22 and 24. That evidence remains
valid only for its exact inputs and does not establish this new workflow's
cross-platform result. Source changes or future public exports need their own
review and qualification. Passing this gate is not publication approval.

## In-progress validation

The workflow policy fixtures pass on Node 22.23.4. They exercise missing path
filters, privileged permissions, unsafe artifact names, rerun collisions, and
silent missing uploads. They also reject changed action pins, missing platform
declarations, an unsupported Node selection, and package-local command paths.
Run only these preparatory cases with:

```sh
node --test --test-name-pattern='^policy fixture' test/package-coexistence-workflow.test.mjs
```

The workflow file is still delegated work. The full test file intentionally
requires it and is not yet qualified. Source and tarball identity rejection
tests will execute its actual validation code after that checkpoint arrives.
These four preparatory fixture results are not a workflow or OS acceptance.
