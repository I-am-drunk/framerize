# Framerize Nexus objective

Maintain the Framerize Nexus package without implementing the parent-owned
Framerize server, accounts, sessions, minting, or inference.

## Ownership boundary

Keep the package in `framerize-nexus/` with its source, tests, packaging
scripts, and documentation. Do not create a server monorepo or add packages
for server accounts, sessions, minting, or inference.

The inherited account and provider implementation is transitional. Preserve
it during package migration until the parent defines and reviews a thin-Nexus
contract. Do not silently remove or expand that implementation here.

## Transport and verification

Preserve existing transport behavior. Use local fixtures for cancellation,
backpressure, streaming, tool calls, and shutdown. Do not make live inference
requests as part of package migration or release verification.

Test the packed package and native Linux x64, macOS arm64, and Windows x64
executables outside the checkout. Record package version, source commit, and
SHA-256 hashes for downloadable release artifacts.

Do not publish or activate a release without owner approval.
