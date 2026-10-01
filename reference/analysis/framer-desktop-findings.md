# Framer desktop app — decompile findings

Artifact: `https://updates.framer.com/electron/darwin/arm64/Framer.zip`
(x64 also published), extracted `app.asar`, version **2026.39.0**, Electron Forge.

## Verdict: thin shell

The desktop app is a BrowserWindow host for the web app. All real logic is remote.
Only Mac builds are published (no Windows/Linux).

## Facts

- Entry `bundle/main.js` (~1.2 MB), plus `preload.js`, local `express` server for
  plugin/dev bridging (`vhost` dep), `node-mac-permissions` for OS permissions.
- Environments (`SERVICE_ENV`): production (`framer.com` / `api.framer.com`),
  `development` (`*.development.framer.com`), `framerlocal` (`*.framerlocal.com`).
- Login window URL: `https://framer.com/login/?origin=framer-app`.
- Session: cookie `session`, domain `.framer.com`, HttpOnly, expiry = now + 365 d,
  stored in partition `persist:framer`. Functions observed: set-cookie-from-value
  (deep-link handoff), remove-cookie (sign out), poll-cookie (detect login).
- URL router classifies navigation into `dashboard` / `project` / `external` /
  `unauthorized` (`/auth/signout`, `/auth/v4/signout`, `/cookie-monster`).
  Project URLs match `^/projects/(?:[\w-]+--)?([A-Za-z0-9]{20})(-[A-Za-z0-9]+)?`
  (same regex as `bootstrap.js`; the optional suffix is a project access token).
- Deep links: `framer-app://` protocol.
- Native bits: `@worklouder/device-kit-framer` (device kit addon), sentry,
  localforage cache.

## What this means for Framerize

Nothing in the desktop app is required. The whole product surface — login,
projects, agent chat — is the web endpoints documented in `framer-auth-flow.md`
and `framer-ai-endpoints.md`. A Framerize account session only needs the `session`
cookie + the ability to mint Castle tokens in its own webview.
