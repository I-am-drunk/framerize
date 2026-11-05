# Session authentication

`framerize-server/framer-auth` supplies session authentication for the Framer
account backend. The CLI creates it for the selected JSON account when run
with `--backend framer`. The generic HTTP backend, Remote and shared core do
not use it.

The account owner supplies a session cookie in memory. This module neither
reads a browser profile nor stores cookies or tokens on disk.

```ts
import { createFramerAuth } from 'framerize/framer-auth';

const auth = createFramerAuth({ sessionCookie });
try {
  const token = await auth.accessToken({ signal });
  await sendAuthorizedRequest(token, signal);
} finally {
  await auth.close();
}
```

Use one instance per account session. Replacing a session requires closing
the old instance and creating another. Concurrent callers share one request;
one caller's cancellation does not cancel the others. The last cancellation
and `close()` stop the request and wait for its client connection to close.

The example's `sessionCookie`, `signal`, and `sendAuthorizedRequest` belong
to the account runtime. Never pass a cookie or token in CLI arguments or logs.

## Request and cache rules

The only request is `GET https://api.framer.com/auth/ai/access-token`, with
cookie authentication. Responses must contain `accessToken` and a valid
RFC 3339 `expiresAt`. The cache refreshes 30 seconds before expiry and never
retains a token for more than ten minutes. `invalidate(rejectedToken)` clears
only that token, so a late rejection cannot evict a newer token.

The default total deadline is 15 seconds. Response headers are capped at
16 KiB without header-count truncation; bodies are capped at 64 KiB.
Requests ignore environment proxies. Redirects, retries and decompression
are disabled. Errors contain fixed messages and codes, never response bodies.
TLS certificate verification stays enabled even when the host environment
requests insecure TLS.

Only a 401 from this endpoint reports `session_unavailable`. A 403 reports
`auth_refused`; it does not mark an account signed out or identify the cause.
Other failed responses report `auth_unavailable`. Cancellation, shutdown and
deadlines have separate codes. This module does not change account state.

## Integration and verification

The protocol and cache rules follow the existing
[account authentication implementation](https://github.com/I-am-drunk/framerize/blob/16b8117950621fccb648108796ca2a7a8c720543/crates/framer-api/src/auth.rs).
The TypeScript implementation uses Node HTTP directly and has no relay path.

Synthetic loopback tests cover acquisition, cache isolation, invalidation,
cancellation, cleanup, error handling, limits, proxy isolation and TLS refusal.
An integration test supplies this helper to the real provider's credential
callback and exercises the server listener. It does not prove Framer inference.
The [Framer backend](framer-backend.md) connects this helper and a Castle minter
to the inference routes. The CLI loads the configured account and packaged
minter; embedding callers can supply their own minter. See
[account configuration](framer-backend.md#select-a-json-account) for session
references and CLI selection. This wiring and the synthetic tests do not
establish live provider compatibility.
