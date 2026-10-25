# Native disconnect diagnostic fixtures

Source-only fixture input on exact base
`6043ad8859889f12f9399cdaa5c4390e09789b27`. Only the new
`crates/framer-native/src/service/disconnect_diagnostics_tests.rs` and this
note belong to this slice. All seven cases are UNRUN here.

Root supplies the `offline.request.finished` recorder immediately after
`owned()` returns, with the request's generated diagnostic UUID and existing
safe completion projection. Root also contains recorder panics and owns the
two-line inclusion in `service/offline_serving_tests.rs`:

```rust
#[path = "disconnect_diagnostics_tests.rs"]
mod disconnect_diagnostics;
```

The fixture expects `service::serving::tests` as its parent and uses the
existing synthetic Ports, fixture, start, wait and finish helpers. It does
not depend on a new production helper or touch SDK, account or network state.

Run the `service::serving::tests::disconnect_diagnostics` selection after root
composition. Exact leaf selections:

- `dropped_json_retains_one_terminal_diagnostic`
- `unpolled_sse_retains_one_terminal_diagnostic`
- `polled_sse_retains_one_terminal_diagnostic`
- `dropped_json_preserves_uncertain_cleanup_diagnostic`
- `unpolled_sse_preserves_uncertain_cleanup_diagnostic`
- `polled_sse_preserves_uncertain_cleanup_diagnostic`
- `logger_panics_preserve_request_result_and_cleanup`

Each parent starts only its exact test in the current executable, with a
task-owned temporary directory and a 30-second child deadline. It kills and
joins the child on unwind, propagates failure, and requires a completion
marker so a zero-test filter cannot pass. Logger initialization occurs only
in the child, isolating its OnceLocks from other diagnostic tests in broad CI.

The six disconnect cases assert no terminal row while issuance is held,
including after cancellation, then exactly one matching request-owned row.
Confirmed cleanup retains `Cancelled` and `cleanup_confirmed=true`; uncertain
cleanup retains `Cleanup` and false. No dropped consumer emits `offline.complete`.
Rows are checked for the synthetic account UUIDs, their prefixes, session and
payload markers. Polled SSE covers both its opening and a pending result poll.

The seventh child puts a task-owned regular file at `.logs`. The request must
retain its original authorization refusal and confirmed cleanup despite the
owned recorder panic, and direct `offline.complete` recording must also return.
It does not change filesystem permissions or any service configuration.

The terminal row describes the outcome from `owned()`, not parent-task join,
delivery or upstream acceptance. Fixture assertions retain and await cleanup
separately. No compilation, test, SDK or runtime execution occurred in this lane.
