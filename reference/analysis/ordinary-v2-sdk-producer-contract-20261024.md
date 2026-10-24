# Ordinary V2 producer and joined-close transfer

Source base: `3d52cde09bf4c3e9d1a7b934e3ecbacea755767a`.
Base tree: `043103a7474c4d70bc77c7af1d3ce8cb2d03def1`.
This source lane owns the Python producer only. Transport owns the Rust
protocol/consumer. Runtime 544711e3 owns every test, build and activation.
Accepted prerequisite history: `7f89bd9cfea2f6e8b3ef25a1f84ca85a5ddbfab1`.

The agreed private request `native_device_sdk_seed` carries `account`,
`generation`, `lease`, `user`, and `session_cookie`. The latter is the exact
supplied session header, kept only in memory and the private pipe. The reply
has exactly `account`, `generation`, `lease`, `account_incarnation`, `user`,
`session_id`, `observation_id`, and `seed`. Neither session nor seed enters
logs, public responses, account persistence or the rate ledger.

Session/observation IDs are canonical non-nil UUIDs. Incarnation is a positive
u64. The observed SDK seed has the existing lowercase 32-hex V2 shape,
including version nibble 0..5 and variant nibble 8/9/a/b. Shape alone is not
observation evidence. The existing HTTP device remains a separate value.

The producer observes the existing `WARM_JS` and `MINT_JS` wrapper calls on
the selected primary device's page. The generated wrapper preserves the
original call, receiver, result and retry policy. It uses the admitted resident
logger observer, requires one complete V2 success and no V3/missing result,
and captures the seed without normalizing the HTTP device. There is no new
SDK invocation. A private record retains the original realm, document,
storage, module, callable, wrapper, URL and observation identity.

Verified-session authority comes from the existing `complete` command's
successful `/web/users/me` response and its existing cookie read. The host
retains only the session token digest, verified user and fresh session ID.
Cookie fanout marks a context eligible only after its existing write succeeds.
This reuses the verified cookie result and tracked host writes, without a new
cookie-jar read at close or a claim to observe arbitrary browser cookie changes.
An imported or merely supplied cookie does not establish verified identity.
Pre-login warmups are not relabeled as observations under the later session.
An already-planned successful SDK call after verification is required.

Observation state is private process memory. It is not restored from device
files, enrollment receipts, cooldown ledgers or another process incarnation.

Root explicitly authorized one read-only retained-record validation inside
the existing close owner. This resolves the first contract's missing lifetime
boundary. Its only reply is a fixed `valid` Boolean, with no extracted seed.
It compares the captured JS anchors, host-retained seed and exact private
account/session/incarnation/observation identities. A copied replacement
record fails the original record's identity closure. The internal JS boundary
uses a decimal incarnation string to avoid losing u64 precision.

Validation gets at most one second within the existing remaining ten-second
close deadline. Close proceeds on absence, refusal, exception, timeout or
cancellation. Any outstanding validation task is cancelled and joined after
the same browser close, so validation settlement cannot prevent starting
cleanup. The original outer close timeout and retained RPC owner remain.
Only successful validation followed by that joined close and the original
host lifecycle checks permits transfer. Waiter cancellation revokes the
record before any late completion; failed closure never publishes a proof.
This establishes validation immediately before joined close, not continuous
JavaScript equality or page-script quiescence during the close interval.

The native host attaches the transferred record to its original grant object
under the original lease. Rust keeps its exact original `Arc<DeviceGrant>`.
The existing `native_device_mint` and `native_device_minted` messages add
`session_id` and `observation_id` only for SDK-bound grants. Their replies echo
both identities, including `ready=false`. Admission rechecks the retained
record before and after the existing lock/spacing wait; it adds no read or
rate charge. Joined finish releases the existing mint lock and reports
`ready=false` if the record became stale. Generic close remains available
for cleanup and retains the original risk-refusal cooldown policy.

Session mutation, account incarnation/replacement, observation replacement,
navigation, host restart and changed proxy registration invalidate or refuse
the retained proof. The HTTP device must still match the selected device,
saved state, existing rate owner and original grant. There is no SDK fallback
to the HTTP device. Imported sessions and restart-restored accounts need a
separately authorized verified producer; this input does not invent one.
Acceptance, current cookies and matched native egress remain independently
required by their existing owners. This record is not Framer acceptance.

Package contract: `native_device.py` imports global module
`sdk_seed_observation`, which requires `contextual_authorization` at load and
uses the unchanged resident observer lazily. The prepared package must load
the real helper before importing/installing `native_device` and before
`Host().loop()`. `native_device.install` injects `Host._sdk_seed` and enables
observation. The raw embedded host has no new import and defaults to no helper.
The separately owned package closure must preserve all source hash checks.

Pinned inputs at the common base:

| File | Git blob |
| --- | --- |
| `scripts/camoufox_host.py` | `c4690cf03de93c099e79746c6dd71c2d738d075e` |
| `scripts/native_device.py` | `58fc1c3dec8a0f53f2dc76bbfa37ba7bb27f14fd` |
| `scripts/resident_authorization.py` | `12293ba6085a81418ddd0a102338b326a6a3a202` |
| `scripts/contextual_authorization.py` | `8ec7f3e5a1a85df72304fe47feefd316cf5d6949` |

Qualification remains UNRUN in this source lane. The new
`scripts/tests/native_device/test_sdk_seed_observation.py` contains 23 cases:
17 Python lifecycle/wire cases and six synthetic JavaScript cases. They cover
verification absence, exact session binding, malformed validation, failed or
cancelled close, late join after waiter loss, navigation/replacement, stale
admission, stale joined finish, retained uncertainty and private reply shapes.
The JS cases use synthetic storage, logger and callable objects; they check
original warm/mint results, call counts, copied-record refusal and changed
identity/anchors. They do not load or evaluate the real SDK.

The additive route fixtures build on producer `56284770c1244449383df3d7269e39a69b2ba7a0`.
Two cases cover seven synthetic scenarios at the original reported boundary.
They extract the real `_register_proxy` method and register after joined close,
before native open. A changed revision refuses even if the route is unchanged
or later changed back. A genuinely unchanged registration permits attachment.
Direct in-memory route changes with no revision change also refuse attachment,
both before native open and after open. Refusal emits only the fixed private
rejection, attaches no observation and permits original-grant cleanup without
charging a mint. These are unexecuted fixture expectations, not runtime evidence.
The producer/helper bytes and existing private wire are unchanged by this delta.

Source-only correction after `8b3dd4fdacc91bd2631249da1adce4c48745a927`:
the fixture disposal callbacks now accept the real `(account_object, account_id)`
arguments, including the late-join wrapper. The fixture extracts the unchanged
host `_retain_resident_fence` method and uses the real `ActivityLedger` so failed
or cancelled closure asserts retained ownership; successful closure checks its
absence. The previous one-argument callbacks could not reach those assertions.
The selection remains 23 cases, all UNRUN here. Production pins are unchanged.

Owner entry: add `scripts` and `scripts/tests/native_device` to `PYTHONPATH`,
then select `test_sdk_seed_observation` with the existing isolated unittest
runner. The six JS cases require Node for inert snippets. Existing native
device component cases are relevant compatibility controls. No test, build,
SDK, browser, private-state or live-request execution occurred here.

Next action belongs to runtime 544711e3: combine the producer, Transport's
strict consumer and the separately owned package closure, then qualify the
selected source. The full browserless outcome and goal accounting remain open.
