# Local activation

Verified on November 3, 2026:

- Release: `framer-next-v0.1.5`.
- Source commit: `264a96a73c7cb72b849f54bb0d6d25942181ade7`.
- Binary SHA-256:
  `db40f6180de734432be33f3fa11ccd4cffecb95e4a3a709649e334c32326f65c`.
- Installed binary: `/home/tjninja/.local/bin/framer-next`.
- Service: `framer-next.service`, enabled with `systemd --user`.
- Loopback endpoint: `http://127.0.0.1:42789`.
- Upstream route: `http://127.0.0.1:8787/openai`.

The release artifact passed `sha256sum -c SHA256SUMS`. The installed binary
hash matches that artifact. `/health` returns HTTP 200. This activation does
not claim successful live Framer inference.

The previous binary is retained at
`/home/tjninja/.local/share/framer-next/rollback/framer-next-pre-0.1.5`.
