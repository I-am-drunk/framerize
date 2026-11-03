# Framerize Nexus objective

Rebuild Framerize as a Node-native system. Keep Framerize Nexus as one
component, not the whole repository. Preserve the browser-based implementation
as a maintained fallback and retain the recovered `browserless` history.

## Repository layout

Put the current package in `framerize-nexus/` with its README, tests, packaging scripts, and
release documentation. Add the Node-native session and minting implementation
as separate packages with narrow interfaces. Move code before changing its
behavior, and update imports, package names, workflow paths, and documentation
in the same reviewed migration.

## Runtime and account boundaries

Use Node for the new runtime. Do not call system CTR. Keep account metadata in
JSON and credentials in private files or the existing private session service.
Return allowlisted account metadata and redact secrets from every log. Keep
upstream protocol conversion inside the connector. Test cancellation,
backpressure, stream completion, short and long streams, tool calls, and clean
shutdown with focused fixtures before any live check.

## Verification and distribution

Run fast unit and integration tests on each pull request. Build and smoke-test
Linux x64, macOS arm64, and Windows x64 artifacts for pull requests without
publishing them. Publish immutable artifacts only from reviewed version tags.
Every release records its version, source commit, and SHA-256 hashes. Exercise
the npm package, `npx`, and standalone executables outside the checkout.

Keep the README direct and useful. Document installation, private account
configuration, endpoint setup, logs, release recovery, and known upstream
entitlements. Continue patch and minor releases until the supported platforms
and installation paths pass their full acceptance checks. Preserve old branches
and history rather than treating the new folder as a replacement for them.
