# Framer backend

`framerize-server/framer-backend` connects the shared server listener to
Framer's Responses and Chat Completions routes. It is an opt-in embedding API.
The CLI still selects the generic HTTP provider, not this backend.

```ts
import { createServer } from 'framerize-server';
import { createFramerAuth } from 'framerize-server/framer-auth';
import { createFramerBackend } from 'framerize-server/framer-backend';

const auth = createFramerAuth({ sessionCookie });
const provider = createFramerBackend({ projectId, auth, castle: minter });
const server = await createServer({ serverToken }, { provider });
await server.listen();
```

The application's shutdown handler must await `server.close()`.

The caller supplies the session cookie, project ID, server token and a Castle
minter. These variables are not loaded automatically. Never put credential
values in command arguments or logs. The backend owns the supplied auth and
minter lifetimes. Do not share them between backend instances. Their methods
must honor cancellation and settle after cleanup; `close()` must be idempotent.

## Protocol and limits

Native requests are wrapped in the `agents` envelope. Responses use
`/ai/v3/openai/responses/`; Chat Completions uses `/ai/v3/chat/`. The request's
streaming choice and tool-call fields remain unchanged. Successful response
bytes pass through without parsing SSE or appending a terminator.

Readiness checks session authentication, not a paid inference or Castle mint.
It does not prove the next inference will succeed. Each inference acquires a
token and mints once. There are no retries, account rotations or alternate
feature categories. An inference refusal never marks the session signed out.

The default body limit is 16 MiB. Preparation and response headers share a
15-second deadline. Streaming has a 60-second idle timeout, not a total limit.
Headers retain every pair under a 16 KiB parser limit. TLS verification cannot
be disabled by the environment, and requests do not use environment proxies.
Local authorization, cookies and arbitrary incoming headers are not forwarded.

The envelope follows the pinned Framer request contract in
[`proxy.rs`](https://github.com/I-am-drunk/framerize/blob/16b8117950621fccb648108796ca2a7a8c720543/crates/framer-api/src/proxy.rs).
Only synthetic loopback behavior is tested here. The account loader, CLI
selection, real encoder delivery and live browserless compatibility remain
unfinished. No claim about the cause of the previous live 403 is made.
