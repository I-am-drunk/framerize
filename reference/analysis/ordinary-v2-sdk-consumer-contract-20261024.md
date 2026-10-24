# Ordinary V2 SDK proof consumer

Branch: `fix/ordinary-v2-sdk-consumer-20261024`.
Exact parent: `3d52cde09bf4c3e9d1a7b934e3ecbacea755767a`.
Parent tree: `043103a7474c4d70bc77c7af1d3ce8cb2d03def1`.
This is a delta over root's current-plus-V2 assembly, not a replacement tree.
The missing producer boundary recorded in `7f89bd9` remains historical evidence.
This consumer does not itself observe a browser or establish native acceptance.

The contract was agreed with Structure, which owns the Python producer.
Private `native_device_sdk_seed` input has exactly the identity fields
`account`, `generation`, `lease`, `user`, and `session_cookie`.
The cookie field carries the already supplied private Session header.
The reply has `account`, `generation`, `lease`, `account_incarnation`, `user`,
`session_id`, `observation_id`, and `seed`. IDs are canonical non-nil UUIDs;
incarnation is a positive u64. Seed is the exact observed lowercase 32-hex
V2 value, with version nibble 0..5 and variant nibble 8/9/a/b.

`native_device_mint` and `native_device_minted` carry and echo `session_id`
and `observation_id` in addition to their existing fields. The former reply
has five fields, the latter four. Wrong/missing IDs or malformed replies are
unconfirmed. A joined stale observation returns confirmed=true, ready=false.
The original `native_device_close` protocol remains available to release
the original lease after stale-proof refusal. Non-SDK shapes remain intact.

Root selected one retained-record validation inside the existing close owner,
immediately before the original close and within its remaining deadline.
It checks the retained module/callable/storage/observation and exact
host/account/session/incarnation bindings, returning fixed facts only.
There is no extra seed extraction, SDK call, mint, retry, or delay. Missing,
changed, cancelled, timed-out or exceptional validation prevents transfer but
must not skip close or join. Publication additionally requires successful join
and unchanged host/session/incarnation/observation. This means successful-call
evidence validated immediately before joined close, not continuous JavaScript
equality or quiescence. Pre-login and imported/set_cookie-only sessions refuse.

Rust captures the Session already supplied to CredentialSource, before the
existing authorization call. It does not reread the private session store.
NativeSdkSeed retains the original Arc<DeviceGrant>, exact login, host and
lease, incarnation and observation IDs. V2SeedBinding additionally retains
the original AccountScope. Neither type logs or serializes private material.
Missing proof refuses before worker launch. HTTP device and rate ownership
stay separate; there is no production HTTP-device fallback for V2.

Admission rechecks the proof through the host. After worker join, finish checks
the same proof and awaits the existing host acknowledgement. Cancellation
still joins; failed or lost acknowledgement remains unconfirmed. Restart or
replacement cannot authorize a stale host/lease/observation. The producer owns
its lifecycle checks; no consumer shape check substitutes for them.

New inert selection prefix: `offline_worker::tests::device::v2::seed_handoff::`.
The nine test names are:

- `sdk_transfer_keeps_exact_seed_session_and_original_grant`
- `sdk_admission_requires_original_lease_and_both_proof_ids`
- `sdk_finish_distinguishes_stale_proof_from_unconfirmed_cleanup`
- `sdk_proof_cannot_move_between_account_scope_owners`
- `sdk_transfer_rejects_foreign_malformed_and_incomplete_wire`
- `sdk_missing_or_stale_proof_starts_no_worker`
- `sdk_source_transfer_failure_preserves_cleanup_without_credential_request`
- `sdk_cancelled_transfer_retains_settlement_and_unknown_cleanup`
- `sdk_proof_completion_retains_join_ack_and_stale_refusal`

They cover 26 malformed transfer inputs, exact SDK/HTTP separation, session
and allocation mismatch, separate account-scope owners, receipt echoes,
missing/stale admission, both transfer failure classes, and four completion
outcomes: success, stale proof, cancellation, and lost acknowledgement.
Existing worker and Bridge fixtures provide the child and acknowledgement gates.
The cancellation supplement holds the transfer reply in the existing synthetic
host. Six cases cross explicit cancellation or waiter disposal with a valid
proof, local refusal or unknown transfer result. They require preparation and
account close to await transfer settlement, preserve unknown cleanup, and start
no worker or credential connection. A loopback-only proxy guard prevents an
accidental request from reaching Framer. Production code remains `edda775`;
this supplement changes only the inert handoff fixture and this report.
The two approved legacy setup helpers now supply explicit synthetic seeds.
The five original seed cases remain byte-identical in
`offline_worker_v2_seed_tests.rs`, blob
`20517c15a99ff5066c584e6909580b239289897d`. Their previous qualification is
not a production-observation or new-candidate qualification.

All tests/builds, SDK/browser execution, private reads and live actions are
UNRUN here. Verification is source review, diff checking and blob comparison.
Committed integration candidate `838161646f90f10e543e2a431bf72b17cbcca7f5`
has exact parent `3d52cde09bf4c3e9d1a7b934e3ecbacea755767a`.
Read-only Git comparison confirms all eleven consumer source/fixture paths,
including the five original seed cases, match this lane at `83bab713`.
The producer/helper code matches `56284770`; package code matches `9e966b07`.
This verifies source composition only, not publication, compilation or execution.
Next action: runtime owner 544711e3 qualifies intake's candidate and a freshly
prepared explicit host package. The full native acceptance, account isolation,
matched-egress, cooldown and joined-cleanup outcome still needs runtime evidence.
No service, eligibility, marker, frozen R16 or startup change is made here.
Structure's opt-in native installer supplies the helper; its prepared package
must include `sdk_seed_observation.py` alongside `native_device.py`.
The ordinary embedded host stays independent; no embedding expansion is needed.

The matched-egress interval reported in `b2ee4f9`, after joined close and before
native open, is covered in published producer
`56284770c1244449383df3d7269e39a69b2ba7a0`, on the same `3d52cde` base.
`sdk_seed_observation.py::_Observation` captures the host-owned proxy revision
and effective proxy; `bound()` requires both unchanged. `attach()` reaches
that guard through `_available()`. `native_device.py` rechecks the same proof
before and after mint admission wait and at joined finish. This covers route
replacement without new wire fields or browser reads. The Rust implementation
is unchanged.

This is source coverage, not executed qualification. Producer follow-ups
`8b3dd4f` and `50e04cc`, included in the candidate, extend the original 21-case
inventory to 23 and correct fixture callbacks to the current close contract.
`test_preopen_route_registration_checks_retained_observation` and
`test_effective_route_change_without_revision_refuses_attachment` add seven
synthetic scenarios before native open or proof attachment. They cover revision
changes, effective-route changes, restoration of an earlier route, and unchanged
registration. The existing post-attachment replacement case remains.
All producer cases and the combined transfer remain UNRUN here; the new cases
do not supply executed pre-open refusal evidence. Native acceptance is unverified.
