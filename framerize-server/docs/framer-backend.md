# Framer backend

`framerize-server/framer-backend` connects the shared server listener to
Framer's Responses and Chat Completions routes. Use it as an embedding API or
select it in the CLI with `--backend framer`.

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

## Select a JSON account

`createAccountBackend` from `framerize-server/account-backend` loads an account
and its private session reference before creating the backend. For example:

```json
[
  {
    "id": "personal",
    "projectId": "your-project-id",
    "session": { "env": "FRAMER_SESSION" }
  }
]
```

Use `session: { "file": "private/session" }` for a private file instead.
Relative references resolve beside the JSON file, not the working directory.
The JSON stores references, never cookies or bearer tokens. Session files are
regular files limited to 16 KiB, with no group or other permissions on Unix.
Windows file ACLs remain the operator's responsibility.
One final LF or CRLF is allowed in files. Invalid UTF-8 and BOMs are rejected.

```ts
import { createAccountBackend } from 'framerize-server/account-backend';

const provider = await createAccountBackend({
  accountsFile: '/srv/framerize/accounts.json',
  accountId: 'personal',
  createMinter: ({ accountId, signal }) => makeOwnedMinter(accountId, signal),
});
```

`makeOwnedMinter` is supplied by the embedding application. It must return a
fresh Castle minter for that account. No shared global minter or token cache
is created. The backend closes the returned minter and its own authentication
instance. Startup cancellation waits for a late minter to close before failing.

Omit `accountId` only when exactly one account is enabled. Disabled accounts
cannot be selected, and no account rotation occurs after a refusal. Only the
selected account's session is read, once at startup. Reload credentials by
closing the backend and creating a new one. `model`
and `models` remain descriptive metadata, not routing rules.

The CLI creates its own minter from the packaged Castle Library. Provision the
server token and account session through private environment or file references:

```sh
framerize-server serve --backend framer \
  --accounts-file /srv/framerize/accounts.json --account-id personal \
  --server-token-file /srv/framerize/private/server-token
```

`FRAMERIZE_SERVER_BACKEND`, `FRAMERIZE_SERVER_ACCOUNTS` and
`FRAMERIZE_SERVER_ACCOUNT` provide the same selection through the environment.
The Framer backend rejects generic upstream URLs and authorization references.
The native executable embeds the minter and its assets. It extracts them to a
private temporary directory and removes that directory after minter shutdown.

Embedding callers supply the session cookie, project ID, server token and a Castle
minter. These variables are not loaded automatically. Never put credential
values in command arguments or logs. The backend owns the supplied auth and
minter lifetimes. Do not share them between backend instances. Their methods
must honor cancellation and settle after cleanup; `close()` must be idempotent.

## Protocol and limits

Native requests are wrapped in the `agents` envelope. Responses use
`/ai/v3/openai/responses/`; Chat Completions uses `/ai/v3/chat/`. The request's
streaming choice and tool-call fields remain unchanged. Successful response
bytes pass through without parsing SSE or appending a terminator.

The experimental completed-response routes use separate paths:
`/openai/v1/responses` maps to `/ai/v3/responses/`, and
`/anthropic/v1/messages` maps to `/ai/v3/messages/`. They accept only
non-streaming requests and preserve native request fields inside the envelope.
Responses background jobs are rejected. Messages requests require a positive
integer `max_tokens`. Each accepted request makes at most one provider POST.
This is an implementation of the recorded mappings, not proof that the current
provider accepts every model, field or version header.

Successful custom replies must contain complete native JSON. The server checks
the response or message envelope, known content fields and terminal state,
then returns the original bytes. It does not rebuild the entire provider schema,
resume partial output or generate an additional tool response. A valid refusal
or output-limit result stays unchanged. Malformed, unfinished and over-16-MiB
replies fail with a safe error and `x-should-retry: false`.

Readiness checks session authentication, not a paid inference or Castle mint.
It does not prove the next inference will succeed. Each inference acquires a
token and mints once. There are no retries, account rotations or alternate
feature categories. An inference refusal never marks the session signed out.

The default body limit is 16 MiB. Preparation has a 15-second deadline.
Inference then has a five-minute deadline. For custom completed-response routes,
it covers the full reply, including validation. For the existing streaming
routes it ends at response headers. Streaming has a 60-second idle timeout.
Headers retain every pair under a 16 KiB parser limit. TLS verification cannot
be disabled by the environment, and requests do not use environment proxies.
Local authorization, cookies and arbitrary incoming headers are not forwarded.

The envelope follows the pinned Framer request contract in
[`proxy.rs`](https://github.com/I-am-drunk/framerize/blob/16b8117950621fccb648108796ca2a7a8c720543/crates/framer-api/src/proxy.rs).
Synthetic loopback tests cover account selection and the packaged encoder.
They do not prove live browserless compatibility. No claim about the cause of
the previous live 403 is made.
