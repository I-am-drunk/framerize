# Parent comparison wire helper

Source base: `6c8ab52ee715c0fe7a0b71abec2b221eb4c774d8`. The helper is
`scripts/browser_comparison_parent_protocol.py`. Its inert fixture entry is
`python3 reference/fixtures/browser_comparison_parent_protocol_tests.py`.
The implementation owner has not imported the candidate or run the fixtures.

The public API is `send_frame(sock, value)`, `receive_frame(sock)`,
`validate_ready(value, nonce)`, `validate_result(value, nonce)` and
`validate_done(value, nonce)`. Ready returns `None`; result returns a detached
report dictionary; done returns a detached full envelope. Every rejected input
or ordinary I/O exception becomes the fixed `ProtocolError`, without its cause
or context. The caller supplies the socket and owns its deadline. An I/O
failure after a partial frame is never permission to resend an attempt.

Frames match `browser_comparison_ipc.rs`: network u32, compact UTF-8 JSON,
96 KiB outbound and 8 KiB inbound. JSON objects are required. Duplicate keys,
nonfinite numbers, invalid UTF-8/scalars and coercion from non-JSON types fail.
Validation rejects unknown or missing fields at every report level.

Enums match the debug labels emitted by `browser_comparison_report.rs`,
including `Json` and `FramerSecurityEnvelope`, rather than snake_case labels.
`one_shot_control.rs::Failure` supplies the twelve failure labels.
Baseline bounds match `sidecar_observer/comparison/report.rs::parse`:
nullable status 100..599, body 0..65536, nullable counters 0..1, and the full
accepted-baseline invariant. Native status retains its `Option<u16>` range
0..65535 and counts retain unsigned 64-bit bounds for the target's `usize`.
Excess native counts remain visible in failed reports. No HTTP policy is
inferred from that wire range. Refusal policy and basis are copied separately
for each leg; a missing classification is not re-created from status 403.

The validators are stateless. The controller must enforce exactly one
`ready`, then send `start`, then accept one `result`, durably record it before
sending `recorded`, then accept one `done`, all under the same nonce. Reject
repeated or out-of-order replies. A failed done such as `Admission` can arrive
without a result. That is not success or authority to release a hold.

After a normal recording acknowledgement, require `done.failure` to equal
the saved result's failure, and `done.local_pair_success` to equal the following
predicate from `browser_comparison_child.rs::locally_complete`:

```python
b, n = report["baseline"], report["native"]
local_success = (
    report["failure"] is None
    and report["account_closed"] and report["browser_cleanup_confirmed"]
    and report["native_nonempty_text"]
    and b is not None and b["accepted"]
    and n is not None and n["status"] == 200 and not n["failed"]
    and n["complete_json"] and n["local_cleanup"]
    and n["attempts"] == n["authorization_calls"] == n["transport_starts"] == 1
)
```

`comparison::run` can replace the failure with `CooldownPublication` after
the result was sent if recording fails. This forces local success false and
requires retention/reconciliation, even when the saved result was successful.
Keep this downgrade distinct from normal result/done agreement. A missing
result, an unexpected downgrade or inconsistent success must retain the
controller's fence. `OwnerLost` without a result is also a failed outcome.

Preserve both leg reports when enforcing sequence consistency: native facts
require an accepted baseline, and excess/unknown counts cannot certify the
pair. Shape validation deliberately retains typed failed or uncertain facts
so the controller can publish the existing cooldown policy. Neither a valid
report nor local success establishes external admission, descendant teardown
or permission to release exclusion. Those checks remain controller-owned.
