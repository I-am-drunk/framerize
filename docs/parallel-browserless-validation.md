# Parallel browserless validation

The browser service remains the user's running provider. Use a separate
loopback port, account store, profiles, sessions, device/rate authority and
logs for browserless. A copied account record is not separate ownership:
the account and its Framer login must be outside the browser service's scope.
Do not clear a handoff or change browser eligibility to make a test proceed.

The accepted startup and routing fixtures use empty stores. At 2026-10-23
16:07 UTC, the owned instance on port 8788 also passed the native dispatch
check. It has `eligible-accounts=none`; it is not ready for authenticated
inference. The 256 MiB empty-instance limit is not a validated worker budget.

Before an authenticated run, bind the separate account and its legitimate
session, project context, existing device identity and cooldown state. Use
the private session input mechanism; keep credentials out of arguments and
reports. Verify every writable path is inside the isolated root and keep
the browser paths masked. Match executable, host, support, worker and routing
hashes to the approved inputs, then size the worker from saved measurements.

Use one owned generation and at most two model requests. The first stream
must demonstrate actual response text before its client disconnects, with
the hello HEAD check while that stream remains open. Retain the live response
socket so disconnect still works after HTTP headers detach it from the
connection. Confirm cleanup before the next request. The second must produce
nonempty text, one terminal `response.completed`, clean HTTP EOF and no error.
HTTP 200, headers and heartbeat frames alone do not establish delivery.

Record request count, timing, bounded resource samples and cleanup evidence.
Do not save response text or raw provider error details. A missing lifecycle
event does not prove that no attempt occurred. The previous browser client's
`ai.post.started` and `ai.chain` checks cannot be assumed to observe native
routing. Bind native lifecycle evidence before claiming current-work cleanup
or absence of another attempt. Preserve refusal results and do not retry them
automatically. Keep account, streaming and memory acceptance separate.
