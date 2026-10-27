# Reviewed alpha releases

Changes enter a topic branch and a PR. Do not automatically merge or push
directly to `main`, `browser-based`, `browserless`, or the integration branch.
`main` remains the browser default. An experimental package is not a default
switch, and a release is not a production deployment.

Build versioned applications with `python3 scripts/cargo_with_identity.py build
--locked --release -p framerize`, or select `framerize-native`. The entry point
refreshes Git identity before Cargo checks its cache. Both applications then
report their version and full source commit with `--version`. A dirty build is
labelled. Plain Cargo builds remain usable for development but report unverified
source and cannot be packaged. Packaging executes `--version` and refuses a
binary whose version, commit or dirty state differs from the source tree.
The archive retains its full-commit name and checksums. `build-info.json`
adds the version and intended release URL; the URL alone is not proof that
the release has been published.

## CI and review

`Build applications` is configured for PRs and maintained-branch pushes.
Its application jobs need `FRAMERIZE_PRIVATE_CI_READY=true` and the
`framerize-ci` environment. Leave the variable unset until the runner owner
verifies platform-side admission. The source predicates exclude fork builds,
but a PR can edit them, so they are not a security boundary. Neither an
environment name nor these docs creates required reviewers or fork restrictions.
Verify those controls separately. If the plan cannot enforce them, do not enable
PR jobs on a private runner; use trusted manual dispatch or hosted isolation.
`Check` runs on a GitHub-hosted runner and fails when an application job fails,
is cancelled or is skipped. Fork aggregate checks never need the private runner.

Require an independent review and resolve findings before a human merges.
AI findings do not grant approval. Require the `Check` status in GitHub branch
protection when repository permissions and plan support it. Policy text does
not enforce a server-side merge restriction by itself.

## Publishing

1. Merge the reviewed version change and obtain a successful maintained-branch
   CI run. The next version is `0.6.0-alpha.1`; increment it for later artifacts.
2. Dispatch `Draft an alpha release` from `main` with that CI run ID. It binds
   the successful run attempt, jobs, artifact IDs and API digests. Artifact names
   include the run and attempt. It downloads those IDs and verifies the ZIP
   digests and both packages. Extra package files are refused. The publisher
   rechecks the snapshot and uploads only the explicit verified file list.
3. Review the draft and publish it as a prerelease. Do not mark it latest.
   Record separate live-acceptance limits. Production activation remains manual.

The workflow refuses an existing tag and does not rebuild, replace artifacts,
merge branches, restart services or restore account/history data. A failure
after tag creation can leave a tag without a draft; inspect it before recovery.
Manual dispatch requires this workflow on the default branch. Review that
infrastructure change separately from any browserless default promotion.
The draft job uses a fresh GitHub-hosted runner with the
`framerize-alpha-release` environment. Configure its approval independently.
Small hosted jobs and artifact storage consume GitHub allowances. Heavy builds
remain on the iMac. No environment, runner or repository setting is configured
by committing these workflows. No actual CI or release is qualified here.
