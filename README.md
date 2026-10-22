# Framerize

`main` remains the browser-based service. The `browser-based` branch preserves
that path, and `browserless` contains the experimental native path. See
[architecture and branch policy](docs/architecture.md) and
[current development status](docs/development-status.md) for validated results
and remaining work. A source push does not deploy or switch the running service.

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

## Engines (`--engine`)

Castle scores the browser fingerprint itself (verified 2026-10-02 by diffing
against a known-good environment: headless containers present SwiftShader
WebGL, 800x600 screens, hardwareConcurrency=1 — all container tells). The
engine is therefore swappable:

| engine | what it is | when |
|---|---|---|
| `camoufox` | stealth Firefox sidecar (C++-level spoofing, no CDP artifacts) — the configuration that passed Castle in production | **recommended** — `scripts/linux/install-camoufox.sh` |
| `full-chrome` | default; desktop Chrome headless=new + stealth flags | works when the fingerprint is good enough |
| `attach` | drive your own already-trusted Chrome over CDP | best possible score, you keep Chrome running |
| `headless-shell` | minimal build | known-bad fingerprint; fallback only |

All AI traffic is sent **in-page** from the account's own browser: Castle
tokens are only accepted when mint and transport share one environment
(verified quadrant matrix in `reference/analysis/verified-captures-2026-10-01.md`).

## Quick start (Linux)

```bash
./scripts/linux/install-camoufox.sh   # stealth engine (recommended)
./scripts/linux/start-daemon.sh       # picks camoufox when installed
framerize login                       # magic-link, interactive
fcd                                   # Claude Code on the free route
```

## Status

Working: in-page auth + inference transport, free long-form chain (unmetered
OpenRouter passthroughs, ~100s window, never-fail ladder), account import,
per-account proxies, rotate-identity. See
[`reference/analysis/framer-ai-endpoints.md`](./reference/analysis/framer-ai-endpoints.md).

## Legal note

Framer's endpoints are private and undocumented; automating them likely violates
Framer's Terms of Service. This project is for accounts you own, at your own risk.
