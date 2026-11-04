# Build the native server from its npm archive

For an existing verified executable, the [Framer Next packager](framer-next.md)
creates a compact recovery folder without rebuilding it. Native CI uploads
that folder in a separate tar archive after the normal binary checks.

Use a clean committed checkout and the pinned development dependencies.
Run these commands from the repository root:

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm ci --prefix framerize-server --ignore-scripts --no-audit --no-fund
npm run build:server
(cd framerize-server && npm pack --ignore-scripts)
npm --prefix framerize-server run binary
npm --prefix framerize-server run binary:package
npm --prefix framerize-server run binary:check
```

Ensure the packed archive is in `framerize-server/` before running `binary`.
In CI, each native job downloads the single producer archive instead of packing.
The typed server target builds core and server, including the inline Castle
runtime. Its dedicated `tsconfig.server-tools.json` excludes Remote.

Before bundling, the builder compares every archived file with the committed
source or freshly compiled output. The exact inventory must match. It resolves
imports only inside that bounded archive, except for Node builtins. Missing,
external-package and computed imports fail rather than falling back to files
in the checkout. Castle uses a static import. TypeScript and esbuild are build
tools, not runtime dependencies.

`build.json` records the archive digest, size and bundle inputs beside the
executable digest. The bundle includes Castle directly, without extracting a
separate runtime or launching an external encoder.
The native archive still contains only the executable and its two licenses.
Packaging and release metadata reject an archive changed
after building. The aggregate checks every platform's package binding.
Existing clean-commit, license, executable and checksum checks remain required.

The workflow tests source on Node 22.12.0, 22.23.4 and 24.13.0 across Linux,
macOS and Windows. Native artifacts use Node 22.23.4. Configured jobs are not
evidence of successful hosted execution.

For a supplied executable and an existing Remote archive, run the bounded
loopback check without rebuilding either input:

```sh
npm run check:native -- /path/to/framerize-server /path/to/framerize-remote.tgz
```

This verifies local transport against synthetic providers, selected JSON-account
authentication, built-in minting and process cleanup. It does not prove
live browserless compatibility, package publication or a different platform.
Development binaries from an uncommitted snapshot are not release artifacts.

## Native account check

The native jobs run `binary:check` before writing release metadata. That check
also runs the extracted executable against a synthetic account endpoint.
It verifies selected JSON credentials, two built-in mints, streamed output,
request cancellation and an empty executable search path. No Remote package,
external encoder, source runtime or network account is used.

The `check` command must exit normally. POSIX signal shutdown must also exit
within the fixture's deadline after serving requests. On Windows, Node's
child-process termination is forced, so the report marks signal cleanup as
`not-tested`. This does not claim a Windows console Ctrl+C test.
Hosted results are still required.
