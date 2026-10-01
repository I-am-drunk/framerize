# ADR-002: Castle tokens come from a per-account WebKit session

Status: accepted

Framer requires a Castle request token on login (`/auth/v4/signin`,
`/auth/v4/continue`) and on every `/ai/v3/*` call. Castle tokens fingerprint a real
browser engine; they cannot be produced by an HTTP client.

Decision: every account owns one isolated, persistent WebKit session (cookie jar,
storage, fingerprint). The daemon reaches it through the `BrowserBridge` port
(`framer-core::port::BrowserBridge`), whose only method mints a Castle token by
evaluating JS in that account's webview:

```js
import("<castle chunk>").then(m => m.a())
```

Consequences:
- "Accounts are devices": fingerprints persist per account and must never mix.
- 403 on `/ai/v3/*` means the risk engine refused THIS engine/session — transient,
  handled as `CoolingDown`, never quarantine (contrast: 401 on
  `/auth/ai/access-token` = genuinely signed out).
- The bridge is also the login UI surface: magic-link email entry happens in the
  account's own webview, matching what Framer's risk engine expects to see.
- Chunk hash (`chunk-5LKSDTQ5.mjs`) is a fragile-layer constant with a runtime
  discovery fallback (scan editor resources for the chunk containing
  `castleSDKV3`).
