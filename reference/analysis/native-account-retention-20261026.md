# Native account connection retention, 26 October 2026

The experimental native service leases an account generation for each request.
After a successful request and confirmed request cleanup, the private-store
bridge retains that generation's HTTP client. It can serve the next request
without constructing another client and connection pool. Authorization and
token issuance still occur for every request through their existing owners.

Reuse requires the same account, user, session bytes, explicit proxy and HTTP
limits. A different configuration cannot replace a held generation. An idle
generation closes and joins before replacement. A full cache evicts an idle
entry; it refuses admission if every entry is held. The cache shares the
native client's existing ten-account bound.

Leases keep active requests out of idle eviction. After the last lease ends,
a one-second sweep closes entries idle for at least 15 seconds. Logout and
account changes retain their existing generation barriers. Store shutdown
stops admission, finishes store jobs and joins retained-account closure.
An unsuccessful service request closes its generation instead of retaining it.
The lease stays active through device cleanup and account-state publication.
Only the final successful outcome permits reuse. Dropping an unfinished lease
closes its generation; this also covers cancellation and panic during finalization.

Shutdown's shared completion includes both the store worker and retained native
generations. Concurrent callers and retries after cancellation observe that same
completion, including a prior cleanup failure.

Different accounts continue to own different HTTP clients. This change does
not alter the TLS backend, HTTP version or authentication protocol. Connection
ownership does not establish distinct browser fingerprints or remote
unlinkability. The browser path remains separate.

Qualification completed on 26 October 2026, 01:40 UTC. All 99 selected tests
passed, with zero failures. The selections in `framer-native --lib` were:

| Selection | Passed |
| --- | ---: |
| `account::sessions` before adding the shutdown module | 26 |
| `account::sessions::lease_shutdown_tests` | 4 |
| `service::serving` | 36 |
| `session_lifecycle_tests` | 33 |

Each selection ran with `cargo test --offline --locked -p framer-native --lib`
and `--test-threads=1`. The fixtures' source-only comments describe their
original handoff; the results above record their subsequent qualification.

The first selection includes all 15 delivered lease cases. The later four
cases cover concurrent shutdown, a cancelled shutdown waiter, persistent
cleanup failure and dropping an unfinished lease. Five service finalization
cases cover successful reuse, device-close failure or panic, failed state
publication, cancellation and abandoning finalization.

The 26-case run preceded only the new shutdown test-module declaration and a
device-cleanup comment correction. Production behavior did not change after it.
Both delivered fixtures retain their exact source bytes. Test execution used
the existing private network, shared execution lock, 1536 MiB memory ceiling,
no swap, one CPU and three-minute deadline. No live account or SDK was used.

No deployment, live acceptance, or measured memory or latency improvement is
claimed for retention. The fresh-minter/browser handoff and a successful live
request remain separate unfinished work. Distinct per-account TLS fingerprints
and remote unlinkability are not established by these account-isolation tests.
