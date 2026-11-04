# Development status

`main` is the current development branch. The server, Remote and shared HTTP
core are work in progress. Use the [source setup instructions](../README.md).
Open pull requests contain changes that are not yet part of `main`.

## What verification covers

Source tests exercise configuration, provider mapping, authentication,
streaming, cancellation and cleanup with synthetic state. Installed-package
checks exercise the built server and Remote outside the development checkout.
Native checks exercise a particular executable and record its source and hashes.

These results apply to the revision, artifacts, runtime and platform actually
tested. A previous passing run does not qualify changed code or a different
executable. A queued CI job is neither a pass nor a failure.

The [package workflow](package-coexistence-ci.md) records producer and consumer
artifacts. The [native build guide](native-server-build.md) describes executable
verification. Historical reports remain evidence for their named revisions.

## Before a release

1. Pass the applicable hosted checks and qualify the exact package and native
   artifacts on their target platforms using the [release gates](package-release.md).
2. Verify live provider behavior using the account owner's
   [authorized single-request procedure](inference-validation.md).
3. Publish the reviewed Remote archive through the
   [publication workflow](remote-publication.md).

Live provider compatibility and publication of this source revision remain
unfinished. Readiness, synthetic provider tests and local native checks do not
complete those steps. Keep credentials in private configuration; ordinary tests
must not import live accounts or make live provider calls.
