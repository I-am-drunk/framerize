# Native account TLS fixture

This local HTTPS server supports the native transport's account-isolation
tests. It uses synthetic credentials and an ephemeral self-signed certificate.
It never calls an upstream service or reads a saved account.

Run the three tests from the repository root:

```sh
cargo test --offline --locked -p framer-native --lib \
  transport::http::tls_isolation_tests:: -- --test-threads=1 --nocapture
```

Python 3.8+, OpenSSL with `req -addext`, and TLS 1.2/1.3 support are required.
The tests start and stop the server themselves. The certificate is trusted
only by explicitly constructed fixture clients; a separate check requires
the ordinary client to refuse it. Production certificate policy is unchanged.

The server accepts HTTP/1.1 JSON POSTs under `/ai/`. Replies echo synthetic
authentication, request data, connection number, request count, negotiated
TLS version and the server's session-reuse observation. `fixture_close: true`
forces a fresh connection for the next request. EOF or `stop\n` requests shutdown.

Limits include 16 active connections, 128 total connections, 512 total
requests, 16 KiB headers and 64 KiB request bodies. Handshakes and requests
have ten-second deadlines; the server has a 120-second lifetime and a
five-second cleanup deadline. Temporary key files are removed before listening.

No observed resumption does not prove isolation of a working session cache.
These tests also do not establish browser fingerprints, remote unlinkability,
live token acceptance or production memory savings. See the
[recorded result](../../../reference/analysis/native-tls-isolation-20261026.md).
