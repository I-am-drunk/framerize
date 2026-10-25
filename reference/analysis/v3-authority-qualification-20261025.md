# V3 authority qualification, 25 October 2026

Source `52019edc` includes the authority protocol change from `c8a5491a` and
one additional existing open-fence scenario. It negotiates ledger versus
observation authority before V3 chooses its admission path. Observation
authority requires authenticated proof; rejected or uncertain transfer never
falls back to generic admission. V2 still requires proof. V3 still supplies
the original saved device to its worker.

All four core and thirty application Rust cases pass, with no selected skips.
They include the new proof path and affected cancellation, host acknowledgement,
account fencing and V2 controls. The original twenty-one Python cases ran:
twenty passed, and `test_negotiated_observation_requires_authenticated_proof` failed at its
history comparison after opening the grant. The difference is an empty
per-device request deque where the expected dictionary had no entry. The
other retained values and device-file bytes matched. That failure is preserved.

The selected test-file delta from `ecd117f00230a4a1f53139814a52b17042abaf51`
seeds an existing unexpired request before verification, observation and joined
browser close, then retains the exact whole-history assertion. The source
owner's documentation delta was excluded. Only that corrected case was rerun
in a fresh isolated attempt and passed. The other twenty Python and all
thirty-four Rust passes are reused. All 55 selected cases therefore pass.

The corrected run completed on 25 October at 02:59 UTC with exit 0 and peak
memory 37,597,184 bytes under the unchanged 128 MiB cap. All 27 input hashes
matched before and after; captured processes, unit and cgroup were absent,
and the shared execution lock was free. No exhaustive transient-PID census
is claimed. Aggregate result SHA256:
`5d3ef5e3653bd0f36172d1f538b080c0face1da779e519537cff004d63884032`.
The correction changes no production or Rust source and needs no rebuild.

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

The matching package was subsequently staged in an isolated serial job using
the unchanged generator and 16 exact source inputs. All 20 runtime files were
validated. The generated host is `f519bb3faeba5ec49ea83aa41fd0c46703b5f2414480206533cea634c5f0d00d`;
the support manifest is `97d906d338e62815e8986479677f0fd5ad8c29a542bda16e02333273b802032e`.
The job completed in 1.37 seconds, peak 61,894,656 bytes, with all 29 inputs
unchanged and cleanup independently collected. An older unexecuted binding
was preserved. The new package remains unlaunched; live V3 and normal repair
acceptance remain separate unfinished actions.
