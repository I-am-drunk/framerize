# Generated native host startup

The unchanged generated host now passes through its real `__main__` entrypoint
with fresh, empty state. Its support manifest and source checks run, its native
ownership hook is installed, and its actual JSON command loop serves requests.
Two status reads report the same absent account and host generation. Between
them, native-open without restored ownership returns an explicit rejection.
Closing stdin produces exit zero. No child process is observed at the check,
and the fresh home and runtime contain no files.

`scripts/check_native_host_startup.py` drives this check in private PID and
network namespaces. Host homes and runtime directories are masked; package
inputs are read-only. The cap remains 128 MiB, zero swap and 32 tasks, with
the shared lock, resource reserves and bounded supervisor unchanged.

The first functional pass, receipt `5386796d`, retained an input-binding gap:
pre-existing standard-library bytecode was not pinned. A separate corrected
run uses fresh absent bytecode prefixes in both Python processes and disables
cache writes. Its receipt `ade7090e` passes, with a 46,804,992-byte peak,
zero swap and no memory or task limit events. All 732 recorded inputs match.
Both jobs are collected; captured processes, units and cgroups are absent and
the shared lock is free. Transient descendant observation is not exhaustive.

This qualifies the explicit system Python 3.12 mode without site hooks. It
does not qualify the installed Camoufox virtual environment, lazy browser
dependencies, the Rust sidecar process owner, real ledger restoration or
cleanup with active work. No account, SDK seed, browser, worker, upstream
request or protected-service operation was used. Native sign-in and successful
mint/inference remain separate. Normal rollout approval remains valid; its
terminal authentication and a fresh positive quiet window are still required.
