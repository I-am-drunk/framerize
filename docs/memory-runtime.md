# Browser memory and service containment

The October 18, 2026 outage included kernel global-OOM kills of Codex processes.
Framerize shares the host with T3, Tailscale, other services, and browser tests.
A cap or reclaim throttle on their shared application slice also affects T3.
Contain Framerize in its own service.

The deployed user-service override is:

~~~ini
[Service]
MemoryHigh=infinity
MemoryMax=10G
MemorySwapMax=1G
OOMScoreAdjust=1000
OOMPolicy=kill
~~~

The 10 GiB ceiling covers the daemon, Python, its Playwright driver, and every
browser child. Swap has a separate 1 GiB ceiling. A cgroup OOM kills this service
group, allowing systemd's existing restart policy to recover it. T3 and Tailscale
are outside that group. This cannot guarantee availability if unrelated services
exhaust the host.

Before launching a browser or adding a page/context, the sidecar checks 90% of
each applicable cgroup memory ceiling and reserves estimates for concurrent
allocations. It also leaves 10% of host RAM available. Under pressure it returns
an explicit allocation error. This is admission control, not a strict 90% usage
cap: already loaded pages can grow. The kernel hard cap remains necessary.
The old shared app.slice throttle and ceiling were removed.

One asynchronous Playwright driver controls separate account browsers.
Processes, contexts, cookies, launch options, and proxies remain separate.
Borrowed pages are returned on errors and cancellation. Concurrent creation of
one device context is serialized, partial launches are closed, account-owned
background tasks are cancelled, and abandoned in-page requests are aborted.
Gateway cancellation releases pending replies; closed sidecar output fails
waiters promptly.

Device requests no longer load an unused ambient editor or eagerly allocate a
fallback mint page. Warm browsers remain resident by default. Optional
FRAMERIZE_IDLE_TIMEOUT_S eviction checks every command's activity and never
closes an account with an active request. Storage isolation is tested; absolute
inability to correlate accounts is not a guarantee these changes can provide.

The 100 MB total target for ten accounts and a 10x improvement were not achieved.
A small local page, without customer profiles or vendor requests, measured:

| Measurement | Separate drivers | Shared driver |
| --- | ---: | ---: |
| One account, PSS including Python and browser children | 459.95 MiB | 462.95 MiB |
| Two accounts after local protocol probes, PSS | 1084.02 MiB | 979.13 MiB |
| Two accounts, process count | 16 | 15 |
| Local protocol round trip, median of 10 probes | 17.312 ms | 14.345 ms |

This single paired fixture run measured about 9.7% less total PSS for two
accounts. It does not establish production latency, a ten-account footprint,
or an improvement of that size on every workload. RSS sums double-count shared
pages, so the benchmark records PSS and whole-cgroup memory instead.
The benchmark stops before its test budget or the host reserve is exhausted.

Regression checks use fixtures:

~~~sh
python3 -B -m unittest discover -s scripts/tests -p 'test_camoufox_runtime.py'
node --test scripts/tests/test_browser_jobs.mjs
cargo test --offline -j 1 -p framer-host --lib
~~~

Run scripts/bench_memory.py inside a separate systemd service with MemoryMax,
MemorySwapMax=0, and RuntimeMaxSec. It supports baseline and shared modes.
The October 18 runs used a 1536 MiB hard cap and a 1200 MiB admission budget.
An independent 64 MiB allocation probe was killed inside its own service;
Framerize, T3, and Tailscale remained running with unchanged process IDs.

For deployment, install the binary and Python override atomically, then issue
one systemctl --user restart framerize.service. Never split this into a stop
followed by a later start: Framerize may supply the agent performing the work.
Memory limit updates can be applied live when they do not fall below current
usage. Keep credentials in the existing private configuration. Inspect selected
systemctl properties instead of dumping units or process environments.
