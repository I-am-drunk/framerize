# Releases

A release starts from a reviewed commit on `framer-next`. Run release commands
from `framerize-nexus/`. CI must pass its
unit tests, syntax checks, package dry run, development-server smoke test,
and isolated executable smoke test before publication.

1. Update the package version and commit that change.
2. Create a tag matching `framerize-nexus-v<version>`. The `v*` namespace belongs
   to the legacy project and must not publish this package.
3. Let the release workflow build and smoke-test native Linux x64, macOS arm64,
   and Windows x64 executables on Node 22. macOS executables are ad-hoc signed.
4. Verify artifact hashes and the source commit recorded in `release.json`.
5. Keep publication disabled until an enforceable owner-approved path verifies
   the version, source SHA, and hashes. An environment name alone is not
   approval evidence.

The current workflow only assembles and verifies release evidence. It does not
publish to GitHub or npm. GitHub rejects required
reviewers for the private repository under its current billing plan. Upgrade
the plan or make the repository public before enabling owner approval. Historical
Framer Next publication failed with `ENEEDAUTH`. No Nexus registry attempt has
been made. Do not add npm tokens, sessions, or account files to the repository
or workflow logs.

The `framerize-nexus` npm package name has not been checked or claimed. Availability does not
establish ownership. First publication requires the package owner to configure
trusted publishing for this repository, workflow, and environment.

For owner-assisted first publication, authenticate npm locally and run `npm run
publish:package` from the checkout after downloading reviewed artifacts into
`dist/`. The command
rejects existing versions and registry errors, verifies `SHA256SUMS`, then
publishes the reviewed tarball with the appropriate dist-tag.
It also requires the commit in `release.json` to exist in this repository.
The checkout must be clean, with exactly one tarball in `dist/`.
It checks the tarball package metadata and release digest before contacting
the registry.
It does not print credentials or store them in this repository.
This local path cannot provide GitHub Actions provenance. Configure trusted
publishing immediately after first publication, then use a separately reviewed
owner-approved release workflow for later versions.
The local helper explicitly disables provenance outside GitHub Actions. It
enables provenance in GitHub Actions. Automatic publication remains disabled
pending an enforceable owner-approved path.

Each executable archive must include the Node.js license notices and a
SHA-256 checksum. Production activation is separate from artifact publication.
