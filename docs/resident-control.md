# Experimental resident crossover

This code is on `browserless` and is not deployed. The current running owner
cannot gain this protocol through source replacement. Validate the integrated
binary before starting an owner with `--resident-control-socket`. The selected
account must already have an idle, successful browser page in that owner.
The option requires the `native-dispatch` build and `--engine camoufox`.
Offline-worker and offline-routing options cannot share this startup owner.
The integrated daemon now links, and all 31 socket and three CLI cases pass.
The protected running service still has no resident-control socket enabled.

The private client is `scripts/resident_control_client.py`. It sends one
request to the existing Unix socket and prints only fixed diagnostic facts.
It never reads the session store, obtains a token, starts a browser or retries
a request. The daemon owns the browser baseline, separate native token mint,
native send and settlement. Both tokens remain private to that operation.

The socket must have mode `0600` in an existing, canonical `0700` directory
owned by the invoking user. The full socket path must be at most 107 bytes.
The client verifies the connected peer's UID and
the socket identity before writing. Its request file must be a regular `0600`
file owned by that user; a symlink is refused. Keep the file outside Git.

After the runtime owner has prepared the account-specific request:

```sh
python3 -I -B scripts/resident_control_client.py \
  --socket /absolute/private/control.sock \
  --request /absolute/private/request.json
```

The JSON object requires `account_id`, `user_id`, `team_id`, `project_id`,
`conversation_id`, `route`, `feature_category`, `feature_name`, `step`,
`turn_id`, `baseline_turn_id`, `user_prompt`, `timeout_ms`, `token_limit` and
`request`. Use distinct turn IDs. The server accepts `/ai/v3/responses/` and
`/ai/v3/messages/`, validates the inference plan and selects current credentials
inside its retained owner. Do not supply cookies, bearer tokens, Castle tokens,
headers, proxy settings or another page's context. Structured credential fields
are refused recursively; arbitrary free text must also be kept credential-free.

The complete request frame is bounded to 96 KiB. The client retains both socket
directions until the complete report and EOF arrive. Its default wait is 120
seconds, adjustable with `--wait-seconds` from 1 to 180. This client deadline
does not extend the daemon's operation deadline or authorize another attempt.

Exit `0` means a schema-valid report arrived, including reports of refusal.
It does not mean inference succeeded. Exit `2` reports `not_started`; exit `3`
reports `unknown`, including partial writes, disconnection and interrupted
waits. After an unknown result, the runtime owner must recover actual settlement
before further action. Client closure requests cancellation; it does not prove
that browser or native work stopped.

A browser baseline must be accepted before native minting is allowed. Native
HTTP 200 plus a JSON object is transport evidence, not proof of usable model
output, streaming support or memory savings. Preserve those distinctions when
interpreting the fixed report. Final host admission remains fenced by the
current coordinator policy.

The ten client fixtures pass in the 128 MiB isolated runner. Their entry is:

```sh
python3 reference/fixtures/resident_control_client_tests.py
```
