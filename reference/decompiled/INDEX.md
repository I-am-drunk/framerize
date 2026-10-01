# Decompiled Framer client artifacts (captured 2026-10-01)

Raw client code, organized by subsystem. Hashes are per-deploy; see
`reference/analysis/framer-castle-token.md` for dynamic re-resolution.

- `app/web.7Y7XBBXE.mjs` — SPA entry module (framer.com editor)
- `app/bootstrap.ee61f92e.js` — bootstrap loader (project-URL regex lives here)
- `app/chunk-MV5XJZF7.mjs` — AI transport: Gq envelope, /ai/v3/* fetch layer
- `app/chunk-IL7V6RK4.mjs` — provider routing / upstream selection
- `app/chunk-4B2T242G.mjs` — AI client (panel state, model pinning)
- `auth/login.36HT4Q4D.mjs` — login bundle (signin/continue flows)
- `castle/chunk-5LKSDTQ5.mjs` — Castle wrapper module (export `a()` = mint)
- `castle/castleSDKV3-LNGA36I3.mjs` — Castle v3 SDK (obfuscated)
- `models/chunk-DM5CYKH3.mjs` — model registry, routing tiers, context windows
- `models/chunk-WELZ4GZF.mjs` — model id conventions / display names
- `desktop/` — extracted app source from Framer desktop (Electron) v2026.39.0
