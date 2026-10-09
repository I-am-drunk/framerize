# Desktop deep-link session fixation — 2026-10-09

Framer desktop v2026.39.0, `bundle/main.js`. A genuine implementation bug in
the auth handoff, independent of the credit system. Survives reworks.

## The primitive

`framer-app://` is registered as the default protocol client
(`setAsDefaultProtocolClient("framer-app")`). The `open-url` / `second-instance`
/ `process.argv` handler parses the URL and, for the query param `fut`:

    p(m){ jW(m) && _5(m).catch(ln) }

`jW` = `dAe` = only a UUID regex (`Ej`). `_5(v)` then does:

    cookies.set({url:"https://framer.com", name:"session", value:v,
                 domain:".framer.com", httpOnly:true, expirationDate:+1yr})

**Any UUID-shaped string becomes the `session` cookie for the whole
`persist:framer` partition — no server validation, no user confirmation, no
origin check on the value.**

## Why it works

- Real session cookies are UUIDs (`6e449676-dd07-4a99-851a-f000a0b27f74`),
  so a valid session passes `Ej` trivially.
- The app's auth state is derived purely from the cookie's *presence*:
  `(await cookies.get({url:Vn,domain:Ed,name:"session"})).length !== 0`
  flips `isAuthenticated=true` and starts backing all API calls with it.
- `Ej` also accepts `00000000-…` and `ffffffff-…` sentinel UUIDs.

## Attack chain (one click)

1. Attacker mints a session for *their own* Framer account (magic-link login).
2. Victim (desktop app installed) clicks
   `framer-app://mcp-oauth-return?fut=<attacker-session-uuid>`
   from any browser, chat, or email.
3. Victim's app silently plants the attacker's session and flips to
   "authenticated" — the victim is now working **inside the attacker's
   account**, with no indication anything changed.
4. Projects/sites the victim creates or edits are owned by the attacker;
   anything synced lands in the attacker's workspace.

Variant — *session donation for account takeover*: if the victim can be
induced to paste/click a link carrying a session the attacker controls and
then perform a privileged action, the attacker rides the shared session.

## Why it's not the obvious "feature"

The `fut` param exists to hand a session from the website to the desktop app
after a web login (the "open in app" flow). The bug is that the handoff is
unauthenticated: there is no nonce, no origin binding, no user prompt. A
legitimate flow would require the token be freshly issued *and* confirmed.

## Other desktop surface audited (closed)

- **Local update server** (port 3187–4187, express): serves `/{*splat}` only
  when Host matches; downloads `.nupkg`/`.exe` from updates.framer.com and
  **verifies the code signature** before `sendFile`. Not exploitable without
  a valid signature.
- **MCP bridge** (`framer-mcp-*.sock` / named pipe): 256-bit random token in a
  `0600` connection file, directory-ownership + world-writable checks, nonce
  claiming, connection cap. Properly defended.
- **MCP OAuth return** (`framer-app://mcp-oauth-return`): the only expected
  deep-link host; the `fut` bug is reachable on *any* `framer-app://` URL
  because the handler checks `d(m)` (scheme) before `host===r.host`, and the
  `fut` branch runs before the host check matters.
- **`form_data` path**: requires a user-initiated Google-Sheet popup with its
  own origin guard — not a clean primitive.

## Status

Verified statically against the shipped v2026.39.0 bundle. Live confirmation
requires the desktop app (macOS/Windows) — the Linux sandbox can't run it.
The primitive is a design flaw in the auth handoff, so it will persist until
Framer adds a nonce/confirmation to the `fut` flow — exactly the
patch-resistant class requested.

## Renderer→main bridge audit (2026-10-09, closing pass)

The preload exposes two IPC senders to every renderer:
`__targetRepresentingParentFrame.postMessage` → `service-message-to-main`,
and `__framerApp.postMessage` → `"community"`. Plus a full service-RPC
framework (`Tw.on(wc).register(...)`) per webContents.

Findings:

- **`"community"` channel**: `Hj` validator enforces exact
  `{channel:"community", version:1, type:"activity.unseenCount", payload:{count:int}}`
  and `pAe` checks `event.sender===webContents` + senderFrame URL startsWith
  the marketplace/community hosts. Closed.
- **Service-RPC framework**: services registered per-webContents include
  `__ElectronWebRendering__` (writeToDisk, openSaveDialog, writeToClipboard,
  getImageData), `__ElectronProjectV2__` (openPlugin, selectNode),
  `__ElectronApp__` (window/project management). The router's only guard is
  `t.sender===this.webContents` — **no origin check on what is currently
  loaded in that webContents.** This is the classic Electron confused-deputy
  shape.
- **Why it does not fire**: plugins run as iframes inside the framer.com
  page (no separate webContents), `<webview>` is globally blocked
  (`will-attach-webview → preventDefault`), and every webContents is created
  with `sandbox:true, contextIsolation:true, nodeIntegration:false`. Iframe
  content cannot reach `ipcRenderer`; only the top-level framer.com page can.
  Navigation handlers (`will-navigate`/`will-redirect`/`setWindowOpenHandler`)
  route external URLs to `shell.openExternal` (https/mailto only).
- **Residual risk**: if any framer.com page can be made to navigate the
  project webContents to attacker content (an open redirect on framer.com),
  that content inherits the full service-RPC surface including
  `writeToDisk`/`openPlugin`. The writeToDisk path writes through a
  snapshot-manager `saveFiles` queue with a dialog — worth a live check on
  desktop.

## Confirmed webPreferences (all project/app WebContentsViews)

`sandbox:true, contextIsolation:true, nodeIntegration:false, webSecurity:true,
allowRunningInsecureContent:false, safeDialogs:true, preload:bundle/preload.js`.
The hardened baseline is why the fut cookie-plant (pure main-process, no
renderer needed) is the clean primitive: it needs no renderer compromise.
