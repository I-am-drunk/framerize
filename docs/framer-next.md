# Framer Next recovery folder

Framer Next keeps a standalone copy of the browserless Node server. It uses the
same executable, not a second backend. Node and the Castle runtime are already
embedded. The destination needs no compiler, global Node installation or
service manager.

The packager copies three verified native files and adds a disabled JSON
account example, a short runbook and integrity records. It never copies existing
accounts, credentials, logs or runtime state. It does not start the server.

## Create a recovery copy

Use a verified per-target native output directory and the trusted SHA-256 of
its `release.json`. Do not treat a hash calculated from an untrusted download
as proof of its origin.

```sh
npm run package:next -- /path/to/native-output MANIFEST_SHA256 /new/path/framer-next
```

This compiles packaging tools but does not rebuild the native executable.
The output must not exist. An existing instance is never overwritten.

The folder contains exactly eight files. The Windows executable has a `.exe` suffix.

```text
framerize-server
LICENSE
NODE-LICENSE
accounts.example.json
README.md
native-release.json
recovery.json
SHA256SUMS
```

## Start the saved server

Open the generated README. Copy the example to `accounts.json`, set the real
project ID, and enable only the intended account. Provision the referenced
private files separately. The example never contains a usable credential.

The generated commands bind to `127.0.0.1:8789`. They use the embedded runtime
directly. Ctrl+C stops the process; nothing registers a background service.
Existing host environment settings can still affect options not set explicitly.

## Integrity and CI

The packager compares the executable and licenses with the pinned native
manifest before copying them, then verifies the copied bytes. It preserves
that manifest as `native-release.json`. `recovery.json` names the original
server commit and the supplied-file hashes. `SHA256SUMS` covers seven payloads.

Native CI creates `framer-next-<target>.tar.gz` and its SHA-256 sidecar after
the existing native smoke checks and release metadata. The tar archive
preserves executable permissions that a raw artifact-directory download loses.
It is a separate artifact; the existing native aggregate remains unchanged.

Identical inputs produce identical recovery file bytes. Compressed archive
metadata is not promised to match across different hosts or tar versions.
Packaging a copy does not itself qualify its source or establish live
browserless compatibility. Keep the original native evidence with the backup.

Local tests cover the three target layouts, disabled credentials, hash binding,
size limits and refusal to overwrite an existing instance. Hosted recovery
artifacts still require their own successful workflow runs.

The local Linux check restores the actual archive into a path containing
spaces and punctuation, then starts its Node 22.23.4 executable with an empty
`PATH`. Node 22.23.4 and 24.13.0 drivers check help, authenticated readiness,
unauthorized access and shutdown against a synthetic loopback upstream.
No inference or live account is exercised by that recovery check.
