# Core

Core is the private `@framerize/core` package. Server and Remote both bundle
it as their HTTP listener, so they accept the same inference requests. Only the
backend decides where inference runs.

## Included now

- Strict TypeScript request, response, cancellation, readiness, and close types.
- The accepted API-v1 readiness classifier, ported without changing its policy.
- One HTTP instance factory for direct and remote backends, with optional bearer authentication.
- Compiled ESM and declarations, with no runtime dependencies.

`InferenceBackend` carries opaque Buffer streams. It does not parse prompts,
change models, retry requests, or own account state. Cancellation is explicit
on every request. Server and Remote adapters implement the same interface.

The listener serves the two OpenAI bare routes and the experimental
`/openai/v1/responses` and `/anthropic/v1/messages` routes. Protocol conversion
for those routes belongs to the backend. Live provider parity is unverified.

`createInstance({ backend, token })` returns `listen()` and `close()`. It binds
only after `listen()`, on an ephemeral loopback port by default. Set `host` and
`port` explicitly in each application's validated configuration. The instance
owns the supplied backend and calls its `close()` exactly once.

`authentication` defaults to `'required'`. Every route except `/health` then
needs `Authorization: Bearer <token>`. With `token: null` and the default,
those routes always return 401. Set `authentication: 'none'` with `token: null`
to serve them without a bearer token. Combining `'none'` with a token string
fails validation, as do empty or invalid token strings. The returned `server`
retains Node embedding access.

Response bodies may be synchronous or asynchronous iterables of Buffers.
Requests remain asynchronous iterables. Timeout expiry gives already-ready
I/O a poll turn before cancellation, so an event-loop stall alone does not
discard buffered response bytes. Failed backend shutdown rejects with the
fixed message `Backend shutdown failed`.

The listener exposes `/health`, authenticated `/ready`, `/v1/responses`, and
`/v1/chat/completions`. It preserves opaque response bytes and applies stream
backpressure. Client disconnect and shutdown abort the backend's signal.
Backends must observe that signal and release their own resources.

The instance calls the response iterator's `return()` when it discards a body,
including a response delivered after cancellation or rejected before streaming.
This is best-effort cleanup. It does not wait for a stalled iterator to close.
Backends must release resources even when `return()` precedes the first `next()`.

Request headers have a 16 KiB parser cap without header-pair truncation.
JSON request bodies have a byte cap. `headersTimeoutMs` bounds readiness at
15000 ms by default. `inferenceTimeoutMs` separately bounds `backend.forward()`
at 300000 ms, including backend preparation but excluding the response body.
Set the latter explicitly if an older caller used `headersTimeoutMs` to limit
inference. Both accept positive integers up to 2147483647 ms.
Stream reads and client writes have idle deadlines, with no total
deadline on healthy inference streams. `onRequest` receives only generated
request IDs, known routes, status, outcome, and elapsed time, never request data.

Responses preserve the literal header `x-should-retry: false`, unless
`Connection` names it as hop-by-hop. Other values and duplicates are omitted.
This preserves an upstream instruction not to retry. Locally generated
inference timeout responses also include this header because cancellation
does not prove that the upstream performed no work. Upstream responses
without this instruction retain their existing retry behavior. Clients may
ignore the header or retry after connection loss, so this does not guarantee
that an inference request runs exactly once.

The readiness parser does not impose a network byte cap or timeout. Its caller
must bound the complete response before passing bytes to the parser, inspect
HTTP authentication failures, and clean up the connection.

## Build and test

Run `npm ci --ignore-scripts`, `npm run build`, `npm run check`, and `npm test`
from the repository root. Server and Remote bundle this listener into their
packages. Live provider compatibility is a separate check.

Do not publish an npm consumer that depends on this private workspace package.
Bundle the used Core code into that consumer's release output, then test the
installed artifact outside the workspace.
