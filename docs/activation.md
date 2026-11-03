# Local activation

Verified on November 3, 2026:

- Release: `framer-next-v0.1.7`.
- Source commit: `2920e9f730cdbb3d4189cf6415f2e8df321bd393`.
- Binary SHA-256:
  `dc0eaca01dd13eceb7e07f46793e3fd4ef0f264cfcc6007511b7f452e89c75f4`.
- Installed binary: `/home/tjninja/.local/bin/framer-next`.
- Service: `framer-next.service`, enabled with `systemd --user`.
- Loopback endpoint: `http://127.0.0.1:42789`.
- Upstream route: `http://127.0.0.1:8787/openai`.

The release artifact passed `sha256sum -c SHA256SUMS`. The installed binary
hash matches that artifact. `/health` returns HTTP 200. This activation does
not claim successful live Framer inference.

The previous binary is retained at
`/home/tjninja/.local/share/framer-next/rollback/framer-next-pre-0.1.7`.
