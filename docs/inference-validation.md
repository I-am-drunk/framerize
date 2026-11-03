# Single-request inference validation

Use this command only after the account owner approves the endpoint and model.
It sends a real inference request and may incur provider charges. Preparing or
testing this command does not authorize a request against a live account.

## Run once

Use the TypeScript checkout with its development dependencies installed and
`npm run build` completed. This command is repository tooling, not part of the
Remote npm package or the server executable.

Point it at the local Remote listener or an approved server origin. Provision
the matching bearer token in a private file first. Never put token values in
arguments, profiles, documentation or logs.

```sh
node .tools/scripts/check-inference.js \
  --endpoint http://127.0.0.1:8788 \
  --model YOUR_APPROVED_MODEL \
  --token-file /private/framerize/local-token \
  --send-one-request
```

For an existing private environment variable, replace `--token-file FILE` with
`--token-env FRAMERIZE_LOCAL_TOKEN`. Exactly one credential reference is required.
Use the local token for Remote or the server token for a direct server request.
The origin must have no path, query or credentials. Non-loopback origins require
HTTPS. `--help` makes no request and reads no credentials.

## Request limits

The command sends one `POST /v1/chat/completions`, with `stream: true` and the
fixed user prompt `Reply with exactly OK.` It does not probe readiness, follow
redirects or retry. The selected server or Remote may perform its own readiness
checks. The command cannot establish how many requests a downstream service makes.

The total request deadline defaults to 30 seconds. `--timeout-ms N` accepts an
integer from 1 to 60000 milliseconds. The transport has a five-second idle limit,
a 16 KiB response-header limit and a 64 KiB response-body limit. It retains all
parsed header pairs within that byte limit. It rejects compressed responses and
requires HTTP 200 with `text/event-stream`. SIGINT and SIGTERM cancel the request.
These are local limits, not a provider billing cap.

## Read the result

The command writes one JSON object and exits with code 0 on success or 1 on
failure. It reports the HTTP status, elapsed milliseconds and request attempts.
A connection failure can count as one attempt without reaching the server.
Success also reports event, text-byte and response-byte counts. Output never
includes the endpoint, model, credential, remote error body or generated text.

Success requires nonempty text, a `stop` finish reason and a final `[DONE]` event.
It rejects truncated or malformed streams, refusals and tool-call responses.
It does not require the returned text to equal `OK`. This is a text-inference
check, not a quality test, provider-identity check or tool-call qualification.

Example redacted refusal, with illustrative timing:

```json
{"scope":"single-endpoint-text-inference","requestAttempts":1,"status":401,"durationMs":24,"ok":false,"code":"http_refused"}
```

Fixed failure codes distinguish `configuration`, `credential`, `aborted`,
`timeout`, `transport`, `http_refused`, `invalid_stream`, `response_too_large`
and `unsupported_encoding`. Configuration and credential failures make no
request. Investigate a failure before deciding whether to authorize another run.

## Offline checks and remaining work

After the build, run `npm run test:inference`. Its synthetic loopback fixtures
check the CLI, redaction, request count, response limits, deadlines and cleanup.
CI runs these offline tests. It never invokes the live command.

An offline pass is not live browserless acceptance. Even a successful live
response alone does not establish which provider or account produced it,
cross-platform support, reproducible artifacts or release qualification.
Retain the approved endpoint and account identity separately in private records.
