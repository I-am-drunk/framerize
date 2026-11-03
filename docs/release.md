# Releases

A release starts from a reviewed commit on `framer-next`. CI must pass its
unit tests, syntax checks, package dry run, development-server smoke test,
and isolated executable smoke test before publication.

1. Update the package version and commit that change.
2. Create a tag matching `framer-next-v<version>`. The `v*` namespace belongs
   to the legacy project and must not publish this package.
3. Let the release workflow build the package and executable on Node 22.
4. Verify artifact hashes and the source commit recorded in `release.json`.
5. Publish through the protected `npm-release` environment.

The workflow uses npm trusted publishing with OIDC. Do not add npm tokens,
sessions, or account files to the repository or workflow logs.

The npm package name was available when checked, but availability does not
establish ownership. First publication requires the package owner to configure
trusted publishing for this repository, workflow, and environment.

Each executable archive must include the Node.js license notices and a
SHA-256 checksum. Production activation is separate from artifact publication.
