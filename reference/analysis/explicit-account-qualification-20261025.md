# Explicit-account qualification, 25 October 2026

The existing combined selection ran once on source
`1676f57c0db119f03f079b6bc6f5e184a3b49c8d`, which applies the deltas from
`2ac2dafafae0957b023af020be2202a36bdc0f64` and
`66f5cb08634bf92e6887b5a2a3eb2b3bc8800919` over the current root source.
It includes the eighteen delivered base cases and the two header-decode cases.
The existing preparation divides them into eight new cases and twelve controls
to retain the original bounded compilation settings. No unchanged suite was
rerun because of parentage, and signup and Castle research were not prerequisites.

Both test compilations pass. Of twenty executed cases, fourteen pass:
the twelve existing controls plus `undecodable_explicit_account_is_rejected_before_work`
and `wave_refusal_precedes_undecodable_explicit_account`.

These six cases under `operation_ownership::tests::explicit_account_retry::`
fail at `explicit_account_retry_tests.rs:93`:

- `explicit_local_cooldown_never_starts_peer`
- `explicit_risk_refusal_never_starts_peer`
- `explicit_unknown_cleanup_never_starts_peer_or_clears_fence`
- `unbound_local_cooldown_still_repicks`
- `unbound_risk_refusal_still_repicks`
- `unbound_unknown_cleanup_still_repicks_and_keeps_fence`

Each reports `producer did not finish while its response stayed connected:
Elapsed(())`. This locates the observed failure; it does not establish a
production defect. Server received the exact frozen base and logs, with source
scope limited to this fixture in a separate checkout. Preserve assertions,
timeouts and both passing header cases. A necessary production change requires
concrete evidence before expanding that scope.

Compilation retained a 768 MiB limit; tests retained 256 MiB. The original
no-swap, 32-task, one-CPU, deadline, reserve and isolation guards remained.
The two test runs peaked at 12,460,032 and 12,517,376 bytes respectively, with
zero swap and no memory or task limit events. Both have collected terminals,
absent captured processes, absent unit cgroups and a free shared execution lock.
The inherited first compilation has a durable successful outer receipt and
verified artifact, but its former terminal handle is no longer addressable.
Its saved stage PID and cgroup are absent. No exhaustive capture of transient
descendants is claimed. The sandbox retains host PID and read-only filesystem
visibility; these fixtures did not exercise real accounts or services.

Evidence is retained in the investigation's existing `explicit-account-new-r1`,
`explicit-account-controls-r1` and `explicit-account-input-r1` directories.
The aggregate result SHA256 is
`dccb0a1f2d83947f82aae7aceebde0b5d0362bc36f81ebe8184af6f6ef70c3fc`.
The six-failure log SHA256 is
`a2e7c18e604ca532cd14bdd0777fa9174968798f656abf9d322e57245221a7a7`.
Qualification remains failed pending the bounded correction. No deployment,
service change, live request or account operation followed this result.
