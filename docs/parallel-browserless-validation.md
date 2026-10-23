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

Validation covers 40 distinct cases using synthetic inputs and local socket pairs.
They cover framing, actual socket shutdown, cleanup ordering, privacy,
account overlap, resource refusal and replay protection. The capped fixture
run mounts neither service nor account state and has no external network.
The original 38-case run peaked at 19,009,536 bytes. After the queued-close
correction, 12 workflow and runner cases pass, including both request barriers;
28 unchanged results are reused. The new run peaked at 18,964,480 bytes.
These are test measurements, not loaded browserless usage. A pool slot with
status `closing` is not idle even when `close_started` is still false.

Final review `26089a3b` accepts these source and saved-result bindings with
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

The checker limits its own execution to two POST calls. Exclusive request
ownership still requires coordination with other callers. Completion
metadata is not an exhaustive count of transport starts, and a missing
event remains unknown. The accepted event binary is staged; these fixtures
do not establish deployment, authentication or live stream delivery.
