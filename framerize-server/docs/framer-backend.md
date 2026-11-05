# Framer backend

`framerize-server/framer-backend` connects the shared server listener to
Framer's Responses and Chat Completions routes. Use it as an embedding API or
select it in the CLI with `--backend framer`.

```ts
import { createServer } from 'framerize';
import { createFramerAuth } from 'framerize/framer-auth';
import { createFramerBackend } from 'framerize/framer-backend';

const auth = createFramerAuth({ sessionCookie });
const provider = createFramerBackend({ projectId, auth, castle: minter });
const server = await createServer({ serverToken }, { provider });
await server.listen();
```

The application's shutdown handler must await `server.close()`.

## Select a JSON account

`createAccountBackend` from `framerize/account-backend` loads an account
and its session before creating the backend. Write the session cookie inline:

```json
[
  {
    "id": "personal",
    "projectId": "your-project-id",
    "session": "session=..."
  }
]
```

Use `session: { "env": "FRAMER_SESSION" }` to read it from an environment
variable, or `session: { "file": "private/session" }` to read a file.
Relative file references resolve beside the JSON file, not the working directory.
Session values are at most 16 KiB of printable ASCII with no surrounding spaces.
Session files are regular files, with no group or other permissions on Unix.
Windows file ACLs remain the operator's responsibility.
One final LF or CRLF is allowed in files. Invalid UTF-8 and BOMs are rejected.
An accounts file with an inline session holds a live credential. Keep it out of Git.
Server does not check the accounts file's permissions. On a shared machine,
restrict it to your user, for example with `chmod 600`.

```ts
import { createAccountBackend } from 'framerize/account-backend';

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

The CLI creates its own minter from the packaged Castle runtime:

```sh
FRAMERIZE_SERVER_BACKEND=framer \
FRAMERIZE_SERVER_ACCOUNTS=/srv/framerize/accounts.json \
framerize serve
```

`FRAMERIZE_SERVER_ACCOUNT` or `--account-id` selects one of several enabled
accounts. `--backend` and `--accounts-file` override the variables.
Add `FRAMERIZE_SERVER_TOKEN` to require a bearer token from clients.
The Framer backend rejects generic upstream URLs and authorization references.
The native executable bundles Castle directly. It does not fetch the separate
Castle repository or require an external encoder. See the
[native build contract](../../docs/native-server-build.md).

Embedding callers supply the session cookie, project ID, server token and a Castle
minter. These variables are not loaded automatically. The backend owns the
supplied auth and minter lifetimes. Do not share them between backend instances.
Their methods must honor cancellation and settle after cleanup; `close()` must be idempotent.

## Protocol and limits

Native requests are wrapped in the `agents` envelope. Responses use
`/ai/v3/openai/responses/`; Chat Completions uses `/ai/v3/chat/`. The request's
streaming choice and tool-call fields remain unchanged. Successful response
bytes pass through without parsing SSE or appending a terminator.

The experimental completed-response routes use separate paths:
`/openai/v1/responses` maps to `/ai/v3/responses/`, and
`/anthropic/v1/messages` maps to `/ai/v3/messages/`. Clients can request JSON
or streaming replies. The provider request stays non-streaming and preserves
the other native request fields inside the envelope.
Responses background jobs are rejected. Messages requests require a positive
integer `max_tokens`. Each accepted request makes at most one provider POST.
This is an implementation of the recorded mappings, not proof that the current
provider accepts every model, field or version header.

Successful custom replies must contain complete native JSON. The server checks
the response or message envelope, known content fields and terminal state,
then returns the original bytes to non-streaming clients or converts the reply
to protocol events for streaming clients. It preserves refusals and output-limit
results without resuming partial output or generating another tool response.
Messages streams use zero for unavailable token counts. Malformed, unfinished and over-16-MiB
replies fail with a safe error and `x-should-retry: false`.

Readiness checks session authentication, not a paid inference or Castle mint.
It does not prove the next inference will succeed. Each inference acquires a
token and mints once. There are no retries, account rotations or alternate
feature categories. An inference refusal never marks the session signed out.

The default body limit is 16 MiB. `headersTimeoutMs` gives request-body
collection, authentication and Castle minting a shared 15-second preparation
deadline in the Framer backend. The backend starts its `inferenceTimeoutMs`
deadline after preparation. Core starts its own inference deadline when it calls
the backend. Both default to 300000 ms, so core's budget includes preparation
as well as connection, upload and response headers.
Connection setup also retains its own 15-second limit. The CLI's
`--headers-timeout-ms` and `--inference-timeout-ms` options configure both core
and the backend. When embedding, set the corresponding options on the backend
factory. Also set them on `createInstance`, or on `createServer` when using the
server wrapper. Raising only the backend's `inferenceTimeoutMs` leaves core's
300000 ms default in effect.

For `/v1/responses` and `/v1/chat/completions`, the inference deadline stops after
response headers. These routes have no total lifetime limit after headers.
The completed-response routes `/openai/v1/responses` and `/anthropic/v1/messages`
keep a deadline through reply collection, validation and delivery, including
when the client requests a streaming reply. Response bodies also use
`idleTimeoutMs`, a 60-second body-inactivity limit. The CLI option is
`--idle-timeout-ms`. A deadline failure does not retry
the request. Configure Remote's independent forwarding limits separately;
see [Remote timeouts](../../framerize-remote/README.md#endpoints).

Headers retain every pair under a 16 KiB parser limit. TLS verification cannot
be disabled by the environment, and requests do not use environment proxies.
Local authorization, cookies and arbitrary incoming headers are not forwarded.

The envelope follows the pinned Framer request contract in
[`proxy.rs`](https://github.com/I-am-drunk/framerize/blob/16b8117950621fccb648108796ca2a7a8c720543/crates/framer-api/src/proxy.rs).
Synthetic loopback tests cover account selection and the packaged encoder.
They do not prove live browserless compatibility. No claim about the cause of
the previous live 403 is made.
