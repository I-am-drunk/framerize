# AGENTS.md — working in Framerize

Inherited from Notionize, same taste. Read before changing code.

## Glossary

- **account** — one signed-in Framer identity = one `session` cookie + one
  isolated WebKit session. **Accounts are devices**: Castle binds trust to the
  engine fingerprint, so each account keeps its own persistent browser session.
- **workspace / team** — a Framer team (`teamId`, uuid). Credits live here, not
  on the account. Rotation unit.
- **pipeline** — one agent inference turn: mint/refresh AI JWT → Castle token →
  POST `/ai/v3/openai/responses/` → parse SSE → record `X-Framer-AI-Usage`.
- **the fragile layer** — `crates/framer-api`. Everything Framer-specific is there.

## The ways to hurt yourself

1. **Widening the credential boundary.** The `session` cookie lives only in the
   OS keychain and the account's webview. Never in SQLite, never in logs.
2. **Leaking Framer quirks upward.** Castle, JWT refresh, SSE event names, status
   policy — all stay inside `framer-api`.
3. **Killing healthy accounts.** Only a 401 from `/auth/ai/access-token` itself
   means signed-out. 403 = Castle risk refusal → cool down. 402 = credits gone →
   rotate workspace. Never quarantine on 403/429/5xx.
4. **Minting Castle tokens server-side.** Impossible by design; always go through
   the account's own `BrowserBridge` webview.
5. **Testing against live state.** Use fixtures under `reference/`.

## Reference

`reference/analysis/` is the ground truth record: auth flow, AI endpoints, Castle,
credits, desktop decompile, compat mapping. Update it when Framer drifts.
