# Try a pull request build

Open the PR's **Framerize PR build** comment after its `Check` job passes.
The comment installs Server and Remote into separate temporary npm prefixes.
Use Node 22.12 or newer. Run the two printed `--version` commands before testing.
The `framerize` package is Server; `framerize-remote` is Remote. Both provide
`frm` and `framerize`, so each role needs its own prefix and explicit executable.

Every PR requests the package workflow, including documentation changes. The
standard workflow packs both packages once and tests those same bytes on Linux,
macOS and Windows. Failed or cancelled CI does not publish a download comment.
Runs whose `.github` tree differs from the trusted publisher checkout also stay
as Actions artifacts. These packages contain unreviewed PR code; inspect the
changes before running them. Until the publisher
workflow has landed on the default branch, use the CI run's `pr-preview-RUN_ID`
artifact. Actions artifacts expire after seven days.

## Identify the build

Versions have the form `0.1.0-pr.123.456789.g0123456789ab`. The fields identify
the PR, workflow run and first 12 characters of the built commit. Rerunning a
consumer job keeps that version so it can reuse the original producer tarballs.

The comment and JSON build record distinguish the PR head from GitHub's tested
merge commit. The record also contains the source tree, producer attempt, Node
and npm versions, package sizes and SHA-256 hashes. Download `SHA256SUMS` with the
packages to verify the files. Each successful update replaces the previous
download set for that PR; closing the PR removes its downloads. Keep local
copies of the packages and build record when a reproduction must survive cleanup.

These previews are npm tarballs attached to the `framerize-pr-builds` GitHub
prerelease. They are not published to npm and do not change an npm dist-tag.
They contain no native preview executable. A successful package workflow does
not establish live-provider or account compatibility.

## Run two isolated foreground processes

Use the `preview` directory created by the install commands in the PR comment.
Keep the same absolute path in each terminal. Create private state directories:

```sh
mkdir -p "$preview/home" "$preview/server-home" "$preview/remote-home"
printf '%s\n' "$preview"
```

For a loopback-only exercise, start this synthetic upstream in a third terminal.
It has no account, token or external connection:

```sh
node --input-type=module -e '
import { createServer } from "node:http";
createServer((req, res) => {
  req.resume();
  res.setHeader("content-type", "application/json");
  if (req.url === "/v1/models") res.end(JSON.stringify({ object: "list", data: [] }));
  else { res.statusCode = 404; res.end(JSON.stringify({ error: "Preview fixture: models only" })); }
}).listen(18790, "127.0.0.1");'
```

In the Server terminal, replace `preview` with the printed absolute path:

```sh
env -i PATH="$PATH" HOME="$preview/home" \
  FRAMERIZE_SERVER_HOME="$preview/server-home" \
  "$preview/server/bin/framerize-server" serve \
  --backend http --host 127.0.0.1 --port 18789 --upstream http://127.0.0.1:18790
```

In the Remote terminal:

```sh
env -i PATH="$PATH" HOME="$preview/home" FRM_HOME="$preview/remote-home" \
  FRM_REMOTE=http://127.0.0.1:18789 \
  "$preview/remote/bin/framerize-remote" serve --host 127.0.0.1 --port 18788
```

Check readiness from another terminal:

```sh
curl --noproxy '*' --fail http://127.0.0.1:18788/ready
```

The empty environment prevents inherited Framerize configuration and credentials
from selecting a production endpoint. All three listeners bind only to loopback.
If a port is occupied, choose another and update the corresponding address.
To exercise inference, replace the models-only fixture with a compatible test
upstream you own. Do not use production credentials for an unreviewed PR build.

Keep previews in foreground `serve` mode. `up`, `connect`, `pair` and `service`
commands can affect persistent configuration or services and are not part of
this isolated workflow. A separate npm prefix alone does not isolate those
operations. Stop each process in its own terminal with Ctrl+C. After all three
exit, remove only the temporary preview directory you created.

## PowerShell

Download the two tarballs from the PR comment. Replace `SERVER_TARBALL` and
`REMOTE_TARBALL` below with their local paths. Use fresh PowerShell terminals
for testing so environment changes stay separate from your usual shell.

```powershell
$preview = Join-Path ([IO.Path]::GetTempPath()) ('framerize-preview-' + [guid]::NewGuid())
New-Item -ItemType Directory -Force $preview | Out-Null
npm install --global --ignore-scripts --prefix "$preview\server" SERVER_TARBALL
npm install --global --ignore-scripts --prefix "$preview\remote" REMOTE_TARBALL
& "$preview\server\framerize-server.cmd" --version
& "$preview\remote\framerize-remote.cmd" --version
$preview
```

In each process terminal, set `$preview` to that printed path and prepare it:

```powershell
Get-ChildItem Env: | Where-Object Name -Match '^(FRM_|FRAMERIZE_|NODE_OPTIONS$|NODE_USE_ENV_PROXY$|HTTP_PROXY$|HTTPS_PROXY$|ALL_PROXY$|NO_PROXY$)' | Remove-Item
$env:HOME = "$preview\home"
$env:USERPROFILE = "$preview\home"
$env:FRAMERIZE_SERVER_HOME = "$preview\server-home"
$env:FRM_HOME = "$preview\remote-home"
New-Item -ItemType Directory -Force $env:HOME, $env:FRAMERIZE_SERVER_HOME, $env:FRM_HOME | Out-Null
```

Start the synthetic upstream above with Node. Start Server in one terminal:

```powershell
& "$preview\server\framerize-server.cmd" serve --backend http --host 127.0.0.1 --port 18789 --upstream http://127.0.0.1:18790
```

Start Remote in another prepared terminal:

```powershell
$env:FRM_REMOTE = 'http://127.0.0.1:18789'
& "$preview\remote\framerize-remote.cmd" serve --host 127.0.0.1 --port 18788
```

Use `curl.exe --noproxy '*' --fail http://127.0.0.1:18788/ready` to query it.
Stop each foreground process with Ctrl+C before removing its temporary files.

## Publication boundary

The package producer has no release-write permission. A separate workflow runs
default-branch publication code after successful `Check`. It accepts four bounded
regular files from the artifact: two tarballs and two manifests. It verifies the
current PR, merge commit, source tree, run and file hashes before publication.
Before downloading candidate bytes, it requires the tested merge's complete
`.github` tree to match its own default-branch checkout. This checks committed
CI definitions. PR build and test code still runs inside CI and can change its
workspace or results. This check does not establish that code's safety.
Fork runs resolve their PR before entering the same per-PR publication queue as
close events. Downloaded code and tools are never executed by that workflow.

An existing asset can be reused only if its bytes match. The publisher rechecks
PR state after upload, updates one bot-owned comment, and removes obsolete assets
for that PR. A moved or closed PR cannot qualify a different source revision.
