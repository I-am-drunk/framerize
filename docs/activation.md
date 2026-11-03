# Local activation

Verified on November 3, 2026:

- Release: `framer-next-v0.1.6`.
- Source commit: `7a53243c32bdeef5c66b49b46e068a5d7d372cb8`.
- Binary SHA-256:
  `df25623bc36cb7fe5be1ac5444f55313b602f4be909aa4af204353cc31426ce6`.
- Installed binary: `/home/tjninja/.local/bin/framer-next`.
- Service: `framer-next.service`, enabled with `systemd --user`.
- Loopback endpoint: `http://127.0.0.1:42789`.
- Upstream route: `http://127.0.0.1:8787/openai`.

The release artifact passed `sha256sum -c SHA256SUMS`. The installed binary
hash matches that artifact. `/health` returns HTTP 200. This activation does
not claim successful live Framer inference.

The previous binary is retained at
`/home/tjninja/.local/share/framer-next/rollback/framer-next-pre-0.1.6`.
