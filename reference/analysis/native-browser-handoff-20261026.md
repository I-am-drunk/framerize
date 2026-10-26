# Native mint to browser request handoff

The experimental `feat/native-browser-handoff-20261026` branch connects the
native request owner to a shared browser sender. The sender uses the fresh
token and actual device returned by that request's minter. It performs one
inference POST and returns the response through the existing native owner.
This work is not deployed and does not establish live Framer acceptance.

## Contract

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
