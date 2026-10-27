# Reviewed alpha releases

Changes enter a topic branch and a PR. Do not automatically merge or push
directly to `main`, `browser-based`, `browserless`, or the integration branch.
`main` remains the browser default. An experimental package is not a default
switch, and a release is not a production deployment.

Both applications report their version and full source commit with `--version`.
A dirty build is labelled. Packaging executes that command and refuses a
binary whose version, commit or dirty state differs from the source tree.
The archive retains its full-commit name and checksums. `build-info.json`
adds the version and intended release URL; the URL alone is not proof that
the release has been published.

## CI and review

`Build applications` runs for PRs and maintained-branch pushes. It builds and
tests both applications on the isolated iMac Linux runner. `Check` fails when
either application job fails, is cancelled or is skipped. Same-repository PRs
are eligible for this runner. Forks need a separate trusted review process;
their jobs cannot execute automatically on the private runner.

Require an independent review and resolve findings before a human merges.
AI findings do not grant approval. Require the `Check` status in GitHub branch
protection when repository permissions and plan support it. Policy text does
not enforce a server-side merge restriction by itself.

## Publishing

1. Merge the reviewed version change and obtain a successful maintained-branch
   CI run. The next version is `0.6.0-alpha.1`; increment it for later artifacts.
2. Dispatch `Draft an alpha release` with that CI run ID. It verifies both
   packages, reuses their bytes, creates an exact-commit tag and a release draft.
3. Review the draft and publish it as a prerelease. Do not mark it latest.
   Record separate live-acceptance limits. Production activation remains manual.

The workflow refuses an existing tag and does not rebuild, replace artifacts,
merge branches, restart services or restore account/history data. A failure
after tag creation can leave a tag without a draft; inspect it before recovery.
Manual dispatch requires this workflow on the default branch. Review that
infrastructure change separately from any browserless default promotion.
