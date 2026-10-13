# JWT relay

When iris's egress IP is edge-WAF-blocked on `/auth/*` (measured 2026-10-13:
household CloudFront 403 on `/auth/ai/access-token` while datacenter IPs mint
fine and `/ai/*` inference routes stay open), this relay mints Framer AI JWTs
from an unblocked IP and publishes them to `.ops/jwt-relay.json`. The daemon
fetches them (`FRAMERIZE_JWT_RELAY_URL`/`TOKEN`, see `crates/framer-api/src/auth.rs`)
and falls back to local mint on any relay failure.

## Pieces
- `relay_mint.py` — one mint pass. Runs anywhere with clean egress + `RELAY_PAT`.
- `jwt-relay.workflow.yml` — the GitHub Actions form. Copy it to
  `.github/workflows/jwt-relay.yml` to enable (needs a `workflow`-scope PAT to
  push, or paste via the GitHub web UI). `RELAY_PAT` secret is already set.

## The unlimited-IP design
Every Actions run lands on a fresh Azure runner with a fresh datacenter IP, so
cron-per-pass gives effectively unlimited minting IPs; the 355-min loop job is
the always-on backbone and renews well inside the 600s JWT TTL.
