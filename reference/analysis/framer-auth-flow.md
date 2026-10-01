# Framer auth flow — web + desktop

Sources: live capture from a controlled account (2026-10-01), static analysis of
`login.36HT4Q4D.mjs` (login page bundle) and the Electron desktop app
(`app.asar` v2026.39.0).

## Services map

Every framer.com page sets `window.bootstrap.services` (from `/bootstrap.<hash>.js`):

| key | value |
|---|---|
| api | `https://api.framer.com` |
| app | `https://framer.com` |
| canvas | `https://project-<id>.framercanvas.com` (per-project) |
| events | `https://events.framer.com` |
| login | `https://framer.com/login` |

## Sign-in methods

- Email magic link (default), Google OAuth, passkey. No password.
- All sign-in calls carry a **Castle token** (`castleToken` field) — anti-abuse is
  enforced at login too: HTTP 403 → UI error `VERIFICATION_FAILURE`.

## Endpoints (login bundle)

| Method | Path | Purpose |
|---|---|---|
| POST | `/auth/v4/signin` | body `{email, redirect, inviteId, origin, useFallback, forceMagic, castleToken}` → sends magic-link email |
| POST | `/auth/v4/continue` | completes sign-in; the magic-link token is posted here |
| POST | `/auth/v4/signin/google` | Google OAuth exchange |
| POST | `/auth/v4/signin/passkey` | passkey (WebAuthn) |
| POST | `/auth/v3/signup` | account creation |
| GET  | `/auth/v3/check/{email}` | account-exists probe |
| GET  | `/auth/signin` | current session info |
| GET  | `/auth/teams/domain` | domain config (Bearer) |

## Magic-link sequence (observed live)

1. `POST {api}/auth/v4/signin` with the email → Framer emails a link of the form
   `https://framer.com/login#token=<uuid>`.
2. Opening that URL: the login SPA reads `new URLSearchParams(location.hash.slice(1)).get("token")`,
   immediately strips the hash (client-side, so the token never hits their access logs),
   and passes it as `emailLoginToken` into the login app, which exchanges it via
   `POST /auth/v4/continue`. The page then shows a "Sign in to Framer → Continue"
   interstitial; confirming sets the session cookie and redirects to the dashboard.
3. **Tokens expire after 1 hour** ("Your sign-in link expired after an hour").
   A consumed link shows `framer.com/expired/`.

## Session

- One cookie: `session` on `.framer.com`, HttpOnly, ~1 year expiry. That cookie is the
  whole login state — the per-account jar for Framerize is just this cookie.
- Sign-out: `/auth/signout` or `/auth/v4/signout` (desktop treats those URLs as
  "unauthorized" navigation).

## Tokens after login

- `GET {api}/auth/web/access-token` (cookie) → short-lived JWT, scopes `16383`
  (general web API). Auto-refreshed ~90 s before expiry by an `accessTokenRefresher`.
- `GET {api}/auth/ai/access-token` (cookie) → `{accessToken, expiresAt, expiresInSeconds}`,
  **600 s TTL**, scopes `12288` — this Bearer JWT authorizes every `/ai/v3/*` call.
  JWT payload carries userId, email, name (enough for an account label).

## Desktop app (Electron, v2026.39.0)

- Loads the **same web app**: login at `https://framer.com/login/?origin=framer-app`,
  dashboard/projects from `framer.com`. No separate API surface.
- Session cookie stored in Electron partition `persist:framer` on domain `.framer.com`.
- Login-success detection = watching for the `session` cookie; it can also *set* the
  cookie directly (`cookies.set({name:"session", domain:".framer.com", httpOnly, …})`),
  which is how deep links hand a session to the app.
- Custom protocol: `framer-app://` (`setAsDefaultProtocolClient`).
- API host constant: `https://api.framer.com`. Dev overrides exist
  (`SERVICE_ENV=development` → `*.development.framer.com`).
- Implication for Framerize: anything the desktop app can do, a plain cookie jar +
  the endpoints above can do. The desktop app adds nothing except window management.
