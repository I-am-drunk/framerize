# Resident main validation preparation

These files preserve the source of the October 24, 2026 local validation
preparation. They do not launch a compiler, fixture, daemon or browser. They
are not a standalone repository test runner.

Copy them beside the existing reviewed dependency recipes and bounded runner
inputs. The `native-resident-worker-preparation` support directory supplies
the source snapshot and dependency graph. Its saved inputs supply the bounded
runner, receipt checker and source binding. The supplement refuses source
drift, missing successful prerequisites and reused execution directories.

Prepare a new snapshot with:

```sh
python3 -I -B prepare_resident_worker_validation.py \
  --directory native-resident-worker-validation-r3 --snapshot
python3 -I -B prepare_resident_main_validation.py \
  --directory native-resident-main-validation-r3 \
  --base native-resident-worker-validation-r3 --snapshot
python3 -I -B prepare_resident_python_v2.py \
  --directory native-resident-python-v2-r3
python3 -I -B prepare_resident_library_validation.py \
  --directory native-resident-library-validation-r1 \
  --base native-resident-worker-validation-r3 --snapshot
```

After verifying the parent graph's current core, host, API, server and store
builds, prepare one phase with `--phase`: `main-tests-compile`, `main-tests-run`
or `main-link-compile`. Each prepared directory has the bounded
`launch_prepared.py` and `verify_phase.py` entrypoints. Run them sequentially
under the existing execution owner; preserve all resource reserves.

The worker build selects 15 coordinator and two adapter cases. The main
test build selects 31 socket and three CLI cases in actual main.
The Python bundle selects 130 cases plus three JavaScript fixture scripts.
It compares runtime discovery with the exact frozen AST case inventory.
Every invocation requires an explicit fresh directory; older bundles stay
immutable. The separate library supplement selects 33 host cases (28 resident
RPC and five bootstrap) and 17 API cases. Its selected test files are unchanged;
unrelated test modules/functions are disabled only in the disposable copy.
Test-only helpers remain available. Prepare `host-tests-compile` after the core
library passes, and `api-tests-compile` after core and host libraries pass.
Their matching `host-tests-run` and `api-tests-run` phases enforce exact discovery
and use the parent's serial lock. Each compile retains a 768 MiB cap; test runs
retain 256 MiB. None of these preparation commands executes a test.
Only unrelated `cfg(test)` gates change in its disposable fixture. The linked
daemon uses unchanged production source. Exact case discovery, dep-info,
input hashes, resource receipts and cleanup are required for acceptance.
The runner binds only its own temporary directory at `/tmp` inside the sandbox
so socket fixtures use connectable paths. The receipt checker requires that
exact mount; host `/tmp` is hidden and all previous resource limits remain.

Current execution uses worker/main r4 and library r2 after the reservation fix.
The server selection contains eight cases: two existing resident-admission
cases, three reservation regressions, and three existing negative-status cases.
Core and server-test compilation and all eight server cases pass. Python r3
passes 130 cases and three JavaScript scripts; its sources are unchanged.
Worker r5 reuses exact validated r4 outputs and builds API/server without unused
LLVM bitcode to keep archives below the unchanged 32 MiB limit. The explicit
reuse and bitcode-adjustment helpers are archived here. All selected current
Rust cases and the linked production executable now pass; library tests use
library r2 and main uses main r5. The ten private-client cases also pass.
These results do not activate resident control or establish browserless
inference success.
