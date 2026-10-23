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

A fresh account import alone does not satisfy native admission. The current
host requires its generation's verified cooldown restoration and the actual
selected device, with the browser closed and all account work joined. Prepare
that history using only the separately owned account and instance. Retain the
restoration check; do not seed empty limits or copy the browser service's
handoff to make admission pass. Session files use the selected instance's
`profiles/.sessions` directory.

Before an authenticated run, bind the separate account and its legitimate
session, project context, existing device identity and cooldown state. Use
the private session input mechanism; keep credentials out of arguments and
reports. Verify every writable path is inside the isolated root and keep
the browser paths masked. Match executable, host, support, worker and routing
hashes to the approved inputs, then size the worker from saved measurements.

The current source candidate adds `expected_framer_user_id` to the private
account-import JSON. A mismatch refuses after the identity read and before
session storage, workspace actions or account publication. Empty or oversized
expectations refuse before I/O. Existing imports may omit the field; the
isolated setup should always bind it to its independently selected login.
The same candidate skips automatic browser prewarm when native dispatch is
configured. It does not initialize a device or bypass cooldown restoration.
The four new cases and four retained import regressions pass; no installed
application binary has this change yet.
The first validation attempt failed its unit-identity check before compiling;
the second reached the compiler and found an existing test initializer missing
the new optional field. That fixture now supplies `None`. The fresh R3 compile
and eight-case run pass, with exact discovery and serial filters, zero swap
and verified cleanup. Summary `8f46157b` preserves both earlier failures.
Rebuild the application before binding this source to an isolated live plan.

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

Each stream snapshot now includes `request_started_at_unix` for correlation
and monotonic durations in milliseconds: `headers_elapsed_ms`,
`first_text_elapsed_ms`, `terminal_elapsed_ms`, and `reader_finished_elapsed_ms`.
A successful interruption also records `disconnected_elapsed_ms`. The clock
starts before sending the HTTP request, including connection setup. First text
requires a complete parsed text event or text in the terminal response, rather
than headers, heartbeats or a partial frame. These are client observation times,
including scheduling and parsing; reader exit is not server cleanup evidence.
Five new timing cases and 22 existing cases pass in an isolated socket fixture.
No live latency measurement or additional request is implied by those results.
Final review `d9760e73`, verification `9e31218a`, accepts freeze `2a4ca7ad`
and summary `3eab1e9f`: 35 seals, six published paths and 19 tested inputs.
Admission source and enforced test caps are bound, but the prelaunch memory
and disk values were not saved. Fresh resource checks are still required.

The explicit checker entry point is `scripts/check_parallel_native.py`.
It takes `--plan`, `--plan-sha256`, `--report` and `--execute`. The plan
and report paths must be absolute; the report must not already exist.
It never installs a binary, starts a service, changes eligibility or logs in.
Use it only after a separate activation has bound the accepted binary,
host, worker dependencies, isolated paths and account authority.

The plan is JSON with the following fields:

| Field | Required binding |
| --- | --- |
| `unit`, `invocation_id` | Owned browserless unit and exact generation |
| `browser_invocation_id` | Protected browser generation, read only |
| `root` | Canonical root separate from browser database and profiles |
| `files` | Relative binary and runtime paths mapped to SHA256 hashes |
| `support_manifest` | Pinned runtime-sources.json and all its member pins |
| `memory_max_bytes` | Exact isolated cgroup limit, at most 1 GiB |
| `events_path` | Canonical event file under the isolated profiles/.logs |
| `private_input` | Object containing a private path and its sha256 |

Private input stays inside the isolated root, with no group or other access.
It contains only `account_id`, `framer_user_id` and legitimate `context`.
The checker refuses accounts or logins present in the browser store. It
requires exactly that account in the native store and launch allowlist.
Native launch arguments bind the database, profiles, host, worker config,
routing config, one active slot and port 8788. The cgroup must allow zero
swap and at most the selected memory limit, with a task limit of 32.

The first invocation claims a permanent marker for that service generation.
Changing the report name does not enable another attempt. Retain failed
reports and markers. A fresh attempt requires a separately coordinated
generation and plan; this client does not prepare either one.

The checker now also requires `native.http.send` invocation numbers one and
two, plus matching `native.stream.sends` snapshots after each original stream
cleanup. These are cumulative process counts, so an already-used generation
cannot supply this acceptance. An extra send that never produces headers, or
a send event missing from the log, is exposed by the final counter snapshot.
Missing snapshots and mismatched counts prevent the next request or final
acceptance. The existing body, cleanup, idle-pool and generation checks remain.

The new selection passes 25 Python cases and 13 Rust cases. Three new Rust
cases cover cancellation before headers, transport failure without headers,
and requests discarded or rejected before sending. Ten existing stream and
cleanup cases pass unchanged. The old client fails the new hidden-send case.
The first candidate run caught an error-ordering regression and a validator
cap-comparison typo; both are corrected and the failed records are retained.
The actual caps stayed at 128 MiB for tests and at most 1 GiB for compilation, with
zero swap. These results are offline evidence, not authenticated delivery.
The API library and final executable builds pass. Staged binary `b4151ab3`
retains the previously built host and server libraries; selecting the accepted
lazy host still requires the explicit sidecar path. No service was changed.

Final send review `baef93ed` accepts the frozen source and saved evidence,
with verification `3add1bbb`, freeze `d1bdcf10` and summary `31f9210e`.
It covers 132 seals, the 794-file snapshot, the 13 Rust and 25 Python cases,
and the saved library/executable builds. It does not cover live execution.

The earlier checker validation covered 40 distinct cases using synthetic inputs and local socket pairs.
They cover framing, actual socket shutdown, cleanup ordering, privacy,
account overlap, resource refusal and replay protection. The capped fixture
run mounts neither service nor account state and has no external network.
The original 38-case run peaked at 19,009,536 bytes. After the queued-close
correction, 12 workflow and runner cases pass, including both request barriers;
28 unchanged results are reused. The new run peaked at 18,964,480 bytes.
These are test measurements, not loaded browserless usage. A pool slot with
status `closing` is not idle even when `close_started` is still false.

Final review `26089a3b` accepts those earlier source and saved-result bindings with
no findings, closing the original queued-close finding. It binds freeze
`075b12d9` and summary `13bde1a3`; the original rejected review is retained.
This does not approve an account or establish a deployed generation.

The separate `scripts/tests/run_restored_native_host.py` process fixture now
passes six cases against the unchanged generated host and support package.
It restores synthetic account JSON and checks the selected device, existing
cooldown/full-window refusals, retained mint charges after confirmed cleanup,
ownership after uncertain cleanup, changed-account rejection and handoff
replay rejection. The namespace has no external network, real account paths
or service-control socket. It invokes native admission commands, not the SDK.
Direct cgroup measurements peaked at 123,883,520 bytes and seven tasks, with
zero swap/events. All nine fixture processes were joined; its unit and cgroup
were absent afterward. This validates the process wiring, not authentication
or inference. The deliberately uncertain account remains logically unconfirmed;
joining its fixture process does not establish application cleanup.

Final review `b0868885` accepts source and saved-evidence bindings with no
findings. It binds freeze `74c02edc`, summary `987ac63c` and verification
`acf906aa`, covering 26 seals, five published files and 49 synthetic outputs.
The restoration destination is the fixture's Python parent, and mint and
cleanup completions are injected. Child absence is sampled rather than traced
for the entire lifetime. Resource measurements cover these synthetic cases
only; no daemon recovery, account or authenticated inference is accepted.
The existing separate-account selection remains the next activation dependency.

`scripts/tests/run_restored_daemon.py` extends the process check to the real
server binary with full synthetic account records. The restored receipt binds
the actual server and host identities. Repeated missing-session requests use
the normal streaming route and release their device leases on one generation,
with idle pool and hello HEAD checks between them. A full restored rate window
refuses admission. A changed account record remains rejected with no restored
receipt. An inert worker sentinel must remain unused throughout.
Two cases passed in the first run. The JSON-mismatch assertion used the wrong
HTTP status; the route maps `Cleanup` to 502. Only that assertion was corrected
and that case rerun, for three distinct passing cases. Both runs and the exact
unchanged case/helper comparisons are retained. Peak memory was 127,746,048
and 109,379,584 bytes, with 10 and 9 tasks, zero current/peak swap and no memory
events. The fixture reaped its server and adopted host processes; both units
and cgroups were absent afterward. These are synthetic admission/refusal and
cleanup checks, not authenticated streaming or deployment acceptance.

Final review `f299b5c3` accepts this source and saved evidence with no findings,
binding freeze `caf09f97`, summary `cdc939cc` and verification `860a35a5`.
It covers 65 seals, five published files, 13 unchanged production contexts
and 42 synthetic outputs. The two original passes are reused exactly; R1
remains failed overall. No current service, real account, SDK execution or
authenticated delivery is accepted by this receipt.

The deferred browser-import candidate passes those same three server cases
with Camoufox and Playwright directories hidden inside the test namespace.
Five additional cases cover native startup without those dependencies,
deferred browser preparation, retained launch options and cleanup, missing
dependencies, and transport setup. The prior source fails the native-startup
case. No browser is launched in these fixtures; browser launch objects are
inert. The accepted server binary is reused with the candidate host selected
explicitly. Its generated hash is `e09123fe`; support bytes are unchanged.

Final review `35d0b188` accepts the frozen source and saved evidence with no
findings, binding freeze `1e7ef8bc`, summary `a36c1078` and verification
`1eb6de8f`. The memory comparison is one matched raw-host initialization
sample before its startup hook or SDK work. It does not validate an actual
browser launch or total service memory. The staged executable still embeds
the old default; a later activation must select the new host explicitly.

The checker limits its own execution to two POST calls. Exclusive request
ownership still requires coordination with other callers. Completion
metadata is not an exhaustive count of transport starts, and a missing
event remains unknown. The accepted event binary is staged; these fixtures
do not establish deployment, authentication or live stream delivery.
