# Server notes

Use the [server guide](../README.md) for source commands, configuration and
embedding. The [development status](../../docs/wip-status.md) lists project-wide
verification limits and release prerequisites.

The server owns provider accounts, session authentication and Castle. Its
package includes Castle at `dist/castle.js`. Normal builds use the vendored
source without fetching the separate Castle repository. Remote and the shared
HTTP core do not contain Castle or provider accounts.

## Historical evidence

The [native evidence directory](native-evidence/) retains manifests, hashes and
reports for specific earlier revisions. Those records describe their named
source and artifacts, not the current server or an open pull request.

The previous development log remains in Git history. Its ownership assignments,
temporary package exclusions and branch instructions are not current guidance.
Use the [native build guide](../../docs/native-server-build.md) for the current
build and verification procedure. Neither these notes nor historical test
results establish live provider compatibility or release readiness.
