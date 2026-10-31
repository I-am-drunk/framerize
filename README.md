# Framerize

Framerize runs Framer AI for you. You give it Framer accounts you own. It keeps them signed in and sends AI requests through them, so tools like Claude Code and Codex can use Framer's models.

One Rust binary. It runs a daemon, a REST and SSE server, and a small web UI.

## How it works

Framer protects its AI with Castle, a fraud-check script. Castle runs in the browser and asks hundreds of questions: screen size, installed fonts, how a canvas draws, how the mouse moves. It packs the answers into a token. Framer rejects requests when the token looks fake.

We tested what Framer actually accepts. The result: a Castle token is only accepted when it is minted inside the account's own signed-in Framer session, and the request is sent from that same session. A token minted anywhere else fails, even when every answer is real.

So each account runs in its own browser session with its own cookies and fingerprint. The token is minted in that session and the request goes out from it. Nothing about the token is invented.

## The two branches

- `main` is the current service. It is the same code as `browser-optimized`.
- `browser-optimized` holds the technique that passed Castle in production: mint inside the signed-in session, send from that session. Verified 2026-10-31 with two live requests that returned HTTP 200.
- `browser-based` keeps the older multi-account browser pool as a fallback. Use it if the current method breaks.

## Repository layout

- `crates/framer-core`: domain and orchestration. No HTTP details.
- `crates/framer-api`: one small file per Framer endpoint. This is the fragile layer.
- `reference/analysis/`: the reverse-engineering record. Auth, AI endpoints, Castle, credits, the desktop decompile. Read these first.
- `reference/tools/framer-ai-console.js`: a working in-browser client. Paste it into DevTools on a framer.com project page to chat through your own session.
- `compat/`: adapters for Anthropic `/v1/messages` (Claude Code) and OpenAI `/v1/responses` (Codex).

## Quick start (Linux)

```bash
./scripts/linux/install-camoufox.sh   # browser engine
./scripts/linux/start-daemon.sh       # start the service
framerize login                       # sign in an account, interactive
fcd                                   # Claude Code on the free route
```

## Status

Working: in-page auth and inference transport, the free long-form chain, account import, per-account proxies, and rotate-identity. See `reference/analysis/framer-ai-endpoints.md` for the endpoint record.

## Legal note

Framer's endpoints are private and undocumented. Automating them likely violates Framer's Terms of Service. Use this only for accounts you own, at your own risk.
