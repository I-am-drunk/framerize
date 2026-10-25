# V3 authority qualification, 25 October 2026

Source `52019edc` includes the authority protocol change from `c8a5491a` and
one additional existing open-fence scenario. It negotiates ledger versus
observation authority before V3 chooses its admission path. Observation
authority requires authenticated proof; rejected or uncertain transfer never
falls back to generic admission. V2 still requires proof. V3 still supplies
the original saved device to its worker.

All four core and thirty application Rust cases pass, with no selected skips.
They include the new proof path and affected cancellation, host acknowledgement,
account fencing and V2 controls. All twenty-one Python cases ran: twenty pass,
and `test_negotiated_observation_requires_authenticated_proof` fails at its
history comparison after opening the grant. The difference is an empty
per-device request deque where the expected dictionary had no entry. The
other retained values and device-file bytes match. A bounded fixture review
and correction is pending. Full qualification is not yet claimed.

Core, host, store, API and server libraries were rebuilt for the changed
grant metadata. The unchanged worker-owner library is reused by source
identity. The matching application linked successfully in 15.2 seconds:
SHA256 `6a2f0a9355f4e73f2a55846eee8bee831f4044089a05304e5405d8432ad64f11`,
29,084,616 bytes, peak 468,168,704 bytes under the original 768 MiB cap.
An initial host wrapper refused before compilation because it expected
512 MiB while the original host contract required 640 MiB. That attempt is
collected; a fresh binding restored the original limit and passed.

All build and test jobs are collected. Input hashes and output bindings match;
known processes, units and cgroups are absent, and the shared lock is free.
No swap or resource-limit events occurred. This is fixture and build evidence,
not SDK execution, a live sign-in, upstream acceptance or a service rollout.
