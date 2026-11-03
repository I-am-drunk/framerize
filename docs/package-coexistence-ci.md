# Package coexistence CI acceptance

This gate qualifies the installed Nexus and server npm packages together.
The workflow is implemented. Its cross-platform qualification is still pending.
The requirements below are not a claim that macOS or Windows has passed.

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
Producer and each platform's report must have distinct expanded artifact names.
A consumer rerun on attempt 2 must accept the original attempt 1 producer when
its source and payload hashes match. The consumer must not require the
producer's attempt number to equal the current attempt.

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

## Local regression validation

All 30 workflow regression tests pass on Node 22.23.4. They exercise missing path
filters, privileged permissions, unsafe artifact names, rerun collisions, and
silent missing uploads. They also reject changed action pins, missing platform
declarations, an unsupported Node selection, and package-local command paths.
Artifact-name checks expand the matrix identities and reject collisions between
platforms or with the producer, including a removed platform suffix.
Run them with:

```sh
node --test test/package-coexistence-workflow.test.mjs
```

Behavioral tests execute the actual inline producer, input verification and
report scripts against a virtual filesystem. They cover source/run/repository
and event mismatches, missing/duplicate/unexpected package evidence, version,
filename, size and digest mismatches, altered payloads, and substituted checker
reports. They also reject a manifest or checkout changed after verification.
An attempt 2 consumer accepts unchanged attempt 1 producer evidence.

These are script-level tests, not native execution on three OSes.

On 2026-11-03, all eight workflow shell steps also passed locally on Linux at
source `58e202ad385041b8d7b20febbb7420f20b37aef7`, using Node 22.23.4 and
npm 10.9.9. The root suite passed all 40 tests with no skips. Each package was
packed once offline, copied unchanged to the consumer input, and installed
together by the existing checker. The consumer used attempt 2 with attempt 1
producer metadata. Both installed packages retained all 13 expected files.

The tarball hashes matched the retained baseline pair:

- Nexus: `12c6eb43755f16a858d12b150bfde44622298ab2fcf489d03abadba0bd4fe528`.
- Server: `c4282b29d43ee8fa38856cd26a4778db82926548fb329d753e5306924d2c2ff3`.

This local run used a synthetic `local_fixture` event. It did not execute
GitHub checkout/upload/download actions, native builds, or macOS/Windows.
All three hosted consumers still require qualification. Later thin-Nexus
runtime changes require new package evidence.
