# Framerize

Always-on service that automates Framer AI: hold multiple Framer accounts, run agent
inference through them. Forked
from [Notionize](https://github.com/I-am-drunk/notionize); the core is replaced with
the Framer web internals mapped in [`reference/analysis/`](./reference/analysis).

One Rust binary (daemon + REST/SSE server + static web UI). Each account runs in its
own isolated WebKit session (its own cookie jar, storage, and device fingerprint) —
required because Framer's anti-abuse (Castle) binds trust to the browser engine, not
just the cookie. **Accounts are devices.**

## Layout

- `crates/framer-core` — stable domain + orchestration. No HTTP details.
- `crates/framer-api` — **the fragile layer**: one small file per Framer endpoint.
- `reference/analysis/` — the reverse-engineering record (auth, AI endpoints,
  Castle, credits, desktop decompile, compat mapping). Read these first.
- `reference/tools/framer-ai-console.js` — working in-browser client: paste into
  DevTools on a framer.com project page to chat through your own session.
- `compat/` — consumer adapters: Anthropic `/v1/messages` (Claude Code), OpenAI
  `/v1/responses` (Codex).

## Status

Research + skeleton. The endpoint map is live-captured and the in-browser client
works; the Rust core is being built out module by module. See
[`reference/analysis/framer-ai-endpoints.md`](./reference/analysis/framer-ai-endpoints.md).

## Legal note

Framer's endpoints are private and undocumented; automating them likely violates
Framer's Terms of Service. This project is for accounts you own, at your own risk.
