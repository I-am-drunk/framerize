# Native account TLS isolation

On 2026-10-26 at 00:19 UTC, all three focused native TLS tests passed in
2.53 seconds. The branch is `test/native-tls-account-isolation-20261026`,
based on `681a6e89df28e58a15c1ca3db8a3bc802168d950`.

The existing build completed successfully in 68.696 seconds. Qualification
used that binary without rebuilding, in a private network namespace with
the shared execution lock, a 1,536 MiB memory limit, no swap, one CPU of
time, 128 tasks and a 180-second outer runtime limit. Both owned units were
absent and inactive with no main process afterward; the lock was released.
No SDK, saved account, browser, upstream service or deployment was involved.

## Results

- Under both TLS 1.2 and TLS 1.3, each account reused its own connection.
  The two accounts had different connections and retained their own bearer
  credentials, fresh synthetic tokens and request bodies. Default inference
  sent no session cookie.
- Forced reconnection kept the correct account credentials and opened a
  different connection. Neither the original client nor the new account
  resumed a session in either protocol version.
- An ordinary client rejected the self-signed server certificate. A fixture
  client trusting that certificate succeeded. The server's expected
  `SSLError` during the negative case is not a failing test.

The production constructor still starts with an ordinary reqwest builder.
A private builder helper lets the fixture add its certificate while using
the same transport policy, limits and account binding. No TLS backend,
credential policy or serving lifetime was changed.

## What this establishes

The tested native clients keep HTTP connection pools and request credentials
separate. Reusing one account's client avoids a new connection for each of
its requests. This supports retaining an account-owned transport, rather
than mutating the identity of a shared client while a request is active.

The selected Linux build uses reqwest's default native TLS backend. The
returned source analysis found no client-session restoration in this path.
The observed lack of resumption is therefore not a positive control for
session-cache isolation. It also does not prove that the server cannot resume
sessions. A resumption-capable backend would need its own working control,
including receipt of TLS 1.3 tickets before attempting reuse.

These three tests cover completed requests, not concurrent stream ownership.
They do not compare ClientHello fingerprints, establish remote unlinkability,
prove live acceptance, or measure request memory and latency. The earlier
[browser-context measurements](shared-context-isolation-20261026.md) remain
separate evidence and are not a native-transport memory result.

Next: give each retained native transport an immutable account and session
generation, then validate eviction, credential rotation and an active stream
surviving another account's closure before measuring the retained path.

See the [fixture and command](../../scripts/experiments/native_tls_isolation/README.md),
[three tests](../../crates/framer-native/src/transport/tls_isolation_tests.rs)
and [source identities](native-tls-isolation-20261026.json).
