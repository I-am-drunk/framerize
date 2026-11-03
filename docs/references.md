# Implementation references

These projects provide concrete patterns. Framer Next does not copy their code.

- [Hono](https://github.com/honojs/hono): small request handlers, explicit
  response construction, and streaming examples.
- [Undici `lib/web/fetch/body.js`](https://github.com/nodejs/undici/blob/main/lib/web/fetch/body.js):
  standards-based HTTP bodies, cancellation, and Web Stream behavior.
- [WHATWG event stream specification](https://html.spec.whatwg.org/multipage/server-sent-events.html):
  event framing, UTF-8 decoding, comments, and multiline data rules.
- [Node.js single executable applications](https://nodejs.org/api/single-executable-applications.html):
  the official executable packaging process.
- [pkg.pr.new](https://github.com/stackblitz-labs/pkg.pr.new): package preview
  workflows and short-lived installable tarballs.
- [npm trusted publishing](https://docs.npmjs.com/trusted-publishers): OIDC
  publication without a long-lived registry token.

The runtime intentionally uses Node built-ins. This keeps installation small
and avoids a framework lifecycle between the connector and its streams.
