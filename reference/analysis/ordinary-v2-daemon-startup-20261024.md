# Packaged ordinary-V2 daemon startup

The exact native package `6012de1c` starts with its explicit packaged host,
the pinned worker configuration and `--offline-sdk v2`. The new
`scripts/check_v2_daemon_startup.py` uses an empty database and profile tree,
no eligible accounts, and a separate loopback port in private PID/network
namespaces. Host homes, runtime state and the service bus remain masked.

Health and empty-account listing pass. The explicit offline endpoint rejects
a zero-timeout request with `InvalidRequest` before account or SDK work.
The normal `/api/hello` route also passes. The daemon has no observed child
process at collection and exits after SIGTERM. This is collection of an empty
instance, not graceful drain acceptance with active work.

The first checker incorrectly requested `/hello`; its earlier checks passed,
then that final assertion failed. Its collected receipt `dc093246` remains
preserved. A separate corrected run records all four checks explicitly and
passes with receipt `a29173c1`. Peak memory was 28,422,144 bytes under 128 MiB,
zero swap and no memory/task events. The executor uses the supported
`TOKIO_WORKER_THREADS=2` setting. All 1,935 input hashes remain unchanged.

The recorded outer processes, units and cgroups are gone; the shared lock is
free. The external snapshot missed transient descendants, so this does not
claim exhaustive process capture. No SDK call, browser, real account, public
request or protected-service action occurred.

The normal rollout remains authorized, awaiting its terminal authentication
and fresh positive quiescence. Native acceptance additionally needs the valid
session/observation/original-grant chain. Current code establishes that chain
only after a fresh verified sign-in and an observed successful SDK call.
Imported cookies alone do not establish it. The user has been asked whether
to retain that first-run path or add explicit saved-session verification.
