# Framer Next

Framer Next is a small local connector for OpenAI-compatible inference. It
accepts Responses and Chat Completions requests on loopback and forwards them
to the endpoint you configure. It uses Node.js built-ins at runtime, with no
browser, no system CTR, and no old Framerize implementation code.

## Install and run

Node.js 22.12 or newer is required.

```sh
npx framer-next serve --upstream https://your-endpoint.example
```

Set the client base URL to `http://127.0.0.1:8788`. The connector serves:

- `POST /v1/responses`
- `POST /v1/chat/completions`
- `GET /v1/accounts`
- `GET /health`

The upstream URL is an endpoint base. For example, `--upstream
http://127.0.0.1:9000` forwards `/v1/responses` to
`http://127.0.0.1:9000/v1/responses`.

## Accounts

Account metadata is JSON. Sessions, cookies, passwords, and tokens do not
belong in this file. See `examples/accounts.json`.

```sh
export FRAMER_LOCAL_TOKEN='your private credential'
npx framer-next serve \
  --upstream https://your-endpoint.example \
  --accounts ./accounts.json
```

Select an account with `X-Framer-Account`. Otherwise the connector uses the
account matching the requested model, then the first enabled account.

## Reliability

- Request, idle-stream, and body-size limits are configurable in code.
- Client disconnects abort upstream reads and backpressure waits.
- NDJSON logs contain request IDs, status, and duration. Known credential
  fields and bearer values are redacted.
- Shutdown closes active streams and sockets without an emission-close phase.
- Only request content and selected compatibility headers cross the boundary.

## Development

```sh
npm ci
npm test
npm run check
node scripts/smoke.js
npm run binary
```

The executable builder writes `dist/framer-next` without modifying Node.
See `docs/references.md` for implementation references and `docs/release.md`
for publication requirements.

## Status

This is a clean rebuild. It is an endpoint connector, not an account login
system or a Castle implementation. Live behavior depends on the configured
upstream and its authorization.
