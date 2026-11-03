# Releases

A release starts from a reviewed commit on `framer-next`. CI must pass its
unit tests, syntax checks, package dry run, development-server smoke test,
and isolated executable smoke test before publication.

1. Update the package version and commit that change.
2. Create a tag matching `framer-next-v<version>`. The `v*` namespace belongs
   to the legacy project and must not publish this package.
3. Let the release workflow build the package and executable on Node 22.
4. Verify artifact hashes and the source commit recorded in `release.json`.
5. Publish the GitHub release before attempting npm publication. A registry
   authentication failure must not suppress the verified executable archives.

The workflow uses npm trusted publishing with OIDC. GitHub rejects required
reviewers for the private repository under its current billing plan. Upgrade
the plan or make the repository public before enabling owner approval. First publication has not
completed because npm returns `ENEEDAUTH`. Do not add npm tokens, sessions, or
account files to the repository or workflow logs.

The npm package name was available when checked, but availability does not
establish ownership. First publication requires the package owner to configure
trusted publishing for this repository, workflow, and environment.

For owner-assisted first publication, authenticate npm locally and run `npm run
publish:package` from the checkout after downloading reviewed artifacts into
`dist/`. The command
rejects existing versions and registry errors, verifies `SHA256SUMS`, then
publishes the reviewed tarball with the appropriate dist-tag.
It also requires `release.json` to match the checkout source commit.
The checkout must be clean, with exactly one tarball in `dist/`.
It checks the tarball package metadata and release digest before contacting
the registry.
It does not print credentials or store them in this repository.
This local path cannot provide GitHub Actions provenance. Configure trusted
publishing immediately after first publication, then use the release workflow
for all later versions.

Each executable archive must include the Node.js license notices and a
SHA-256 checksum. Production activation is separate from artifact publication.
