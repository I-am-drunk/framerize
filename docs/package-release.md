# Remote release candidates

The candidate workflow does not publish to npm or claim live browserless
compatibility. Server native releases stay separate.

1. A `framerize-remote-v<version>` tag runs the reusable `ci` workflow.
2. Source checks complete before the package-pair workflow builds both
   products and packs each once. It retains compiled verification tools.
3. Nine consumers verify the same archives on Linux, macOS and Windows,
   using Node 22.12.0, 22.23.4 and 24.13.0. They do not rebuild products.
4. The release job checks the manifest, exact package inventories and all
   nine reports, then copies the original Remote archive without repacking.

The coordinator also supports manual dispatch. Core and package workflows
keep their own automatic source triggers; the coordinator adds no duplicate
automatic run. These workflows are unqualified until their hosted runs pass.

## Candidate evidence

`remote-candidate-<run ID>` contains the Remote tarball, `producer.json`,
all consumer reports, checker results and installed-flow logs, plus
`release.json` and `SHA256SUMS`. Source commit/tree, repository, event,
run/attempt, Node/npm versions and hashes must agree before output is written.
A later consumer attempt may reuse an earlier producer from the same run.

The release manifest records `publishable: false` and
`pendingGate: "live_browserless_qualification"`. A successful candidate job
does not bypass that gate. No npm credential or write permission is used.
Stale artifacts, changed bytes, incomplete matrices and version mismatches
fail qualification. Historical package or native results cannot replace
these checks for the changed TypeScript packages.

## Approved publication

The separate [Remote publication workflow](remote-publication.md) promotes the
same bytes only after a committed live review and environment approval. It
defaults to preparation only. No live approval record exists at this checkpoint.
Its local tests do not establish hosted operation or registry publication.

## Local verification

```sh
node node_modules/typescript/bin/tsc -p tsconfig.tools.json
npm run test:packages
npm run test:release
```

These are local policy and synthetic evidence tests. They do not establish
hosted platform results, live inference, or registry-based npm/npx installs.
The installed-pair fixture accepts `REMOTE_TGZ` and `SERVER_TGZ` together to
exercise existing archives without repacking. Set
`FRAMERIZE_PACKAGE_REPOSITORY` to the checkout with pinned development tools
when executing the fixture from a downloaded tooling directory.
