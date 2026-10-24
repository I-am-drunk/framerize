# Experimental resident crossover

The matched browser/native control succeeded on 2026-10-24 at 15:55 UTC.
A normal browser request first returned HTTP 200 with model output. Under one
retained resident lease, the browser baseline then returned an accepted HTTP
200 response and native HTTP returned 200 using its own fresh, unchanged token.
Both bodies completed; native returned a JSON object and cleanup was confirmed.
The complete paired operation took about 2.95 seconds, including its browser
baseline. This is not a measurement of native-only latency.

This proves the matched browser-token/native-HTTP path for this isolated
account and request. The native report does not certify parsed model output,
streaming, tool use, browserless token minting or memory savings. Final host
admission remains fenced as designed; the browser and owning daemon remain
resident. The protected browser service was unchanged.

The prior attempt stopped before either comparison send because our slotted
provenance object could not accept Playwright's callback-cache attributes.
Retained ordinary-function callbacks fix registration and removal. No SDK
change was needed. A separate cookie-order correction permits harmless
reordering of distinct names while preserving duplicate precedence, original
wire headers, values and membership checks.

This code is on `browserless`. An existing owner cannot gain the protocol or
these corrections through source replacement. The successful run used a new
isolated owner after the prior owner's idle state and rate history were
captured. Select an idle, successful browser page in that owner. The option
requires `native-dispatch` and `--engine camoufox`; offline-worker and
offline-routing options cannot share it. The protected running service has
no resident-control socket enabled.

For an isolated owner, select the installed Camoufox virtual environment
explicitly with `FRAMERIZE_PYTHON`. System Python may start the sidecar but
fail its lazy browser import. When the protected state directory is masked,
mount only the required virtual environment read-only through that mask.

The cooldown-restoration controller and sidecar must be able to verify each
other's process identities. An external controller failed this check across
the bubblewrap user-namespace boundary. Running the controller and daemon in
the same isolated namespace allowed restoration to complete. Use a fresh
private socket path after a failed startup; the daemon refuses an existing
socket rather than taking it over. These are local activation prerequisites,
not evidence about Castle or upstream HTTP behavior.

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
