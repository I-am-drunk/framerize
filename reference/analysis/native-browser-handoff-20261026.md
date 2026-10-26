# Native mint to browser request handoff

The experimental `feat/native-browser-handoff-20261026` branch connects the
native request owner to a shared browser sender. The sender uses the fresh
token and actual device returned by that request's minter. It performs one
inference POST and returns the response through the existing native owner.
This work is not deployed and does not establish live Framer acceptance.

## Contract

The next acceptance experiment uses a freshly minted V3 token through this
supplied-token browser sender. The earlier browser-token/native-HTTP success
establishes neither native-minter acceptance nor the cause of the later
direct-native `403/request_blocked`. No saved token or browser remint belongs
in the new experiment. An accepted first leg would support a separately
bound comparison using another fresh token and the native HTTP sender.

The `c16fd24` pair is prepared for that experiment. The current repaired
browser uses the older binary without `/api/ready`; a post-switch return
therefore needs the qualified browser package and its account-history
readiness check. The original invocation remains the pre-switch abort target.
Full read-only eligibility passed at 23:55 UTC on October 26, with seven
active users still observed. No common quiet interval or live attempt was
established by that result. Actual drain, joined cleanup and recovery remain
required before the single-request experiment can produce acceptance evidence.

`FreshCastleToken` carries the minted device through authorization. The
browser path checks account, private session, device and token age before
dispatch. Python seeds the request's cookies through the browser API; the
login cookie is not included in the page-evaluation payload. The browser
checks its origin, device and remaining freshness immediately before fetch.

The optional `browser-transport` Cargo feature enables the explicit development
option `framerize-native serve --browser-transport-script /absolute/path/runtime/supplied.py`.
It also requires the usual database, profiles and worker configuration.
The ordinary browser application and the default native sender are unchanged.
The supplied host needs the existing Camoufox/Playwright Python environment.
This explicit source override is not a new packaged installation mode.

One browser owns a separate context for each active request. The context is
closed with acknowledgement before the response can complete. Cancellation
keeps the original host reply and joins the actual browser task. Native
closure also waits for native response resources to be released.

The sender reuses the acknowledged response queue. Queued and unacknowledged
text share its 131,072 UTF-16-unit bound. The final response is still assembled
in memory and is limited to 8 MiB. Native clients receive that completed
buffer through the existing body interface. This is not upstream streaming.

## Local evidence

The real-browser fixture keeps both production origins and all supplied
sender scripts unchanged. A restricted CONNECT proxy sends only those two
origins to a local TLS server. The fixture runs inside private network and
PID namespaces with synthetic credentials. Its launch adapter selects the
recorded browser executable, omits add-ons and trusts only the fixture's
self-signed test environment. Production certificate verification is unchanged.

The successful fixture used one browser for six requests. It checked an exact
8 MiB UTF-8 response, unchanged token and cookies, status and usage headers,
an HTTP refusal without retry, oversize rejection, overlapping account
contexts, cancellation of one account while its peer remained active, and
deadline cleanup. Context, browser and descendant-process closure passed.

The deadline case first found that a completed exchange's `ResponseDeadline`
was mistaken for an expired wait on cleanup. The correction accepts that
exception only when the exchange task is already done. A still-running
exchange remains unconfirmed. A focused Python regression covers the case.

## Startup and storage corrections

Browser preparation now follows account validation, any required legacy
history migration and the ledger's readiness response. The service starts
accepting requests only after the sender is ready. Four new startup cases
exercise successful ordering, migration refusal/cancellation, preparation
failure/panic and abandoned/cancelled preparation. They use an inert sender
and check the real local ledger's exit and profile-lock release.

The integrated storage stack rejects session directories with shared Unix
permissions and refuses non-regular ownership/session files without blocking
on a writerless FIFO. Native startup applies the existing 16 KiB credential
bound during file reads and saves. Oversized writes preserve the prior file;
other store consumers keep their previous size policy. SQLite account lookup
now distinguishes a missing row from query/type failures. A rejected update
is also checked against the complete previous account record.

These source fixes were supplied in `8258b42`, `18ee745`, `3db45be` and
`5ea6886`. Their seven standalone source files are byte-exact here; the
bounded-store constructor is composed with the startup-order change.

## Qualification on 2026-10-26

| Scope | Result |
| --- | --- |
| Final native startup and credential-target selection | 31 passed |
| Session-file and SQLite library checks | 20 passed |
| Real Rust/Python/browser pipe, including cancelled response | 1 passed |
| Native CLI build with `browser-transport` enabled | Passed |

The native selection completed in 21.15 seconds including compilation; its
tests took 4.77 seconds. The storage run took 8.60 seconds including compilation.
The final pipe run took 5.64 seconds and confirmed that all descendant processes
exited. These are qualification durations, not request latency measurements.
All used bounded, isolated runners. No real account or SDK invocation ran.

Earlier handoff results remain valid for their unchanged relevant sources:
32 transport checks, 18 offline-authorization checks, two service-routing
checks, a result-decoding check, eight Python checks, four JavaScript checks
and the six-request local browser fixture above. The pipe was checked again
after moving preparation into startup. Existing failed attempts are retained
in the private qualification record and are not counted as passes.

Live Framer acceptance, total request memory and upstream streaming remain
unverified. Separate browser contexts do not establish distinct TLS
fingerprints or remote account unlinkability.

## Release package qualification

The current matching pair is staged from
`c16fd245ee4a7c26268ab961319581b644d8965a`. The offline, single-job release
build completed in 151.94 seconds within its 1,536 MiB boundary. All 802
source hashes matched through build, package checks and staging. The
1,264-file worker bundle was assembled without SDK execution. Both archived
binaries match the compiler artifacts; earlier immutable archives remain.

The browser package passed nine ordinary checks plus five checks for usable
history, rejected unusable history, preservation of unknown history, host
shutdown failure reporting and an empty owned process group. The native
package passed eight ordinary checks plus request diagnostics, joined
cleanup and absence of private fields. The checks used synthetic local
state. Staging completed with the isolated unit inactive and no process left.

Neither package was installed or deployed. The native build enables
`browser-transport` and still requires a browser sender. Package qualification
therefore does not establish native mint acceptance, browser-free delivery,
total memory savings or request latency. Package identity remains `c16fd24`
as subsequent documentation and source commits advance.

### Earlier 72886b2 qualification

Both release binaries build at `72886b2e1c3fb48aad5d89b4f0f8488f83d21d9b`.
The first build reached its three-minute runtime bound; an incremental build
finished successfully in 79.69 seconds. All 655 recorded source files match.
The worker bundle was rebuilt offline with 1,264 files and a matching source
stamp. That assembly did not execute the SDK.

The browser return archive passes nine checks covering archive/runtime
verification, host shutdown, invalid startup, UI, profile exclusion, both
termination signals and profile reuse. The native archive passes eight
equivalent checks with its packaged worker inputs. Both use empty synthetic
state and make no upstream request. The archives and a pinned copy of the
supplied-browser runtime are staged without changing the running service.

The browser archive supports history schema 2, providing a compatible return
artifact after migration. First migration still requires genuine exclusive
ownership and the observed 600-second interval. Package success does not
establish that transition or live acceptance.
