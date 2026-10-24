# Resident main validation preparation

These two files preserve the source of the October 24, 2026 local validation
supplement. They do not launch a compiler, fixture, daemon or browser. They
are not a standalone repository test runner.

Copy them beside the existing `native-resident-worker-preparation` support
directory and the immutable `native-resident-worker-validation-r2` snapshot.
That separately owned graph supplies the reviewed dependency recipes, bounded
runner, receipt checker and source binding. The supplement refuses source
drift, missing successful prerequisites and reused execution directories.

Prepare a new snapshot with:

```sh
python3 -I -B prepare_resident_main_validation.py --snapshot
```

After verifying the parent graph's current core, host, API, server and store
builds, prepare one phase with `--phase`: `main-tests-compile`, `main-tests-run`
or `main-link-compile`. Each prepared directory has the bounded
`launch_prepared.py` and `verify_phase.py` entrypoints. Run them sequentially
under the existing execution owner; preserve all resource reserves.

The test build selects the 29 socket and three CLI cases in actual main.
Only unrelated `cfg(test)` gates change in its disposable fixture. The linked
daemon uses unchanged production source. Exact case discovery, dep-info,
input hashes, resource receipts and cleanup are required for acceptance.
The runner binds only its own temporary directory at `/tmp` inside the sandbox
so socket fixtures use connectable paths. The receipt checker requires that
exact mount; host `/tmp` is hidden and all previous resource limits remain.

Status: source preparation, syntax checks and frozen-input preservation pass.
Compiler and test phases are unrun. The prepared supplement does not activate
resident control or establish browserless inference success.
