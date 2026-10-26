# Local shared-context isolation and memory

On 2026-10-26, both local HTTPS fixture modes passed on source base
`ed0ed9b0b58fe2f9daafd5226bfbb14fe9bea8c2`. One browser with two account
contexts preserved the tested account state while using less measured memory
than two browsers with one context each. This is not a production switch.

The fixture checked fresh storage, cookies, localStorage, sessionStorage,
IndexedDB, CacheStorage and distinct HTTPS connection ownership. It switched
accounts while a response was held open, closed the other account during its
own unfinished response, then completed the original stream. Both modes
joined cleanup and left no new processes in their private PID namespace.

At the two-account snapshot, total proportional set size (PSS) was 564,463 KiB
with one browser and 791,533 KiB with two. The difference is 227,070 KiB,
or 221.748 MiB. The reduction is `227070 / 791533 * 100 = 28.687%` for this
pair of fixture snapshots. This includes the Python harness and shared
Playwright driver. There were nine owned processes versus sixteen.

Twelve alternating local requests had median round trips of 6.240 ms with
one browser and 8.301 ms with two. These include automation and HTTPS I/O;
they do not measure a pure context switch, an isolated handshake, or Framer.
One pair of runs does not establish a repeatable latency improvement.

Both runs used Camoufox 152.0.4-beta.31 directly through Playwright 1.62.0,
temporary profiles, a synthetic page and a local self-signed HTTPS server.
The production Camoufox configuration generator, SDK, account data and
upstream services were absent. The network and PID namespaces were private;
each cgroup allowed 2 GiB, no swap, 512 tasks and two CPU cores of time.
No memory or task-limit events occurred. One incomplete process-tree sample
in the separate-browser run was rejected and retried before recording.

PSS snapshots are not continuous peaks. Kernel cgroup counters use different
charging rules and are retained separately in the result. Browser startup,
full application pages, production concurrency and long-lived resource
growth need their own measurements. This does not substantiate the old
3.2 GiB versus 128 MiB comparison for the proposed resident browser.

Earlier runs are retained in the investigation record. The initial fixture
accidentally invoked its last assigned function with a null argument; an
explicit initialization function fixed the unexpected `/null` requests.
The first two-browser attempt failed during startup under a 256-task cap.
The passing attempt recorded 281 to 282 tasks under the 512-task cap. That
is consistent with task exhaustion, but the earlier failure did not preserve
its task-limit counter, so the exact cause is not claimed as proved.

## TLS and account ownership

TLS negotiation belongs to a connection, not to an individual HTTP request.
Selecting an account-specific client can be quick; changing a live
connection's handshake is not such a selection. Connections and resumption
state must remain with their owner while requests and streams are active.
This fixture does not compare ClientHello fingerprints, test TLS resumption,
or prove that accounts cannot be associated remotely. A matching TLS profile
alone is not a unique browser-instance identifier.

At the inspected source base, `NativeHttp::build` creates a separate reqwest
client for its account scope. The serving request opens and closes that
scope. The client uses HTTP/1, at most one idle connection per host, and a
15-second idle timeout. Cloning a client shares its connection pool; it
does not create a new account boundary. These constructors implement no
per-account TLS fingerprint generation. Runtime TLS backend and session
resumption behavior still need direct verification before claiming isolation.

Next: verify the native backend's account-owned connection and resumption
lifetime with local TLS evidence before implementing retained account
transports. Any retained client must respect session changes, account closure,
request cleanup and live streams. The shared-browser fixture alone does not
meet the requested separation of browser fingerprints.

See the [fixture and reproduction command](../../scripts/experiments/shared_context_isolation/README.md)
and [recorded measurements](shared-context-isolation-20261026.json).
