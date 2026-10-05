# The phantom "risk-refused" — full root-cause (2026-10-05)

Symptom: under burst load (~30 concurrent chains), ~27-50% of chains died to
clients with the exact error string `risk-refused` — while ai.post events
recorded **zero 403s anywhere** (493×200, 78×429 in the reference window).

## The mechanism (three stacked bugs)

1. **Single-slot JWT cache thrash.** `token_cache:
   Mutex<Option<(AccountId, AiToken)>>` held ONE global entry. With
   round-robin rotation across 3 accounts, *every chain invalidated the
   previous chain's cached JWT and reminted* at `/auth/ai/access-token` —
   ~1 mint per chain, ~300 mints per burst.
2. **The auth endpoint has a sustained-rate limiter.** Measured after one
   burst: 120/120 subsequent mints → **403** (TTL ~20 min).
3. **transport.rs classified every bare-HTTP 403 as Castle** with
   `elapsed=0` (`classify_403(body, 0)` → `Refusal::Castle` → the exact
   sentinel `risk-refused`). The chains died inside `proxy::forward`'s
   `cached_token(...).await?` — which propagates its error **silently** (no
   ai.request event), so the failure never appeared in ai.post (it never
   reached the browser), and the client received a phantom Castle verdict
   *after* stream-open.

Result: phantom `risk-refused` with zero 403s in any browser-path log, plus
incorrect account-health poisoning (cooldowns) for what was really an auth
rate limit.

## Fixes (all deployed, verified)

- **Per-account JWT cache** (proxy.rs/inference.rs): chains on the same
  account reuse the 600s JWT — mints drop ~99% (300/burst → ~3).
- **Global per-(session,path) token cache** (auth.rs): `web_access_token`
  had the same thrash via credits::usage (minted per call). Both token
  types now mint at most once per 570s per session.
- **transport.rs: `/auth/*` 403s are rate limits, not Castle** — only
  `/ai/v3/*` 403s go through Castle classification. Auth rate limits return
  `retryable 403 on <path>` and never touch account health.
- **freechat double-429 `continue` bug**: the nested 429-retry's inner
  `continue` skipped to the NEXT CHUNK on a second 429, silently dropping
  the chunk's content. Rewritten as a bucket-hop loop that retries the same
  chunk, carries the **effective dialect** forward for response parsing
  (a cross-dialect retry previously got parsed as the original dialect),
  and only exits on success / non-429 / exhaustion.

## Verification

- Realistic load (12 concurrent): **11/12 chains complete, 1 honest 429**
  (labeled correctly, no phantoms).
- Auth endpoint rate limit recovered ~20 min after thrash stopped.
- Saturation load (30-way): honest failures only — 429 workspace ceiling
  and fast device-velocity 403s, all correctly labeled and rotated.

## Related: the day's limit map (all measured 2026-10-04/05)

| Limit | Key | Value | Escape |
|---|---|---|---|
| Metering | per route | SSE metered; non-streaming OpenRouter passthroughs free | the three passthroughs |
| Concurrency | per (route×step×device×model) | ~5/bucket; workspace-wide ~25-30 | step rotation + device pool + account spread |
| Velocity ban | per (account×**device**) | cumulative bucket ~300 reqs, ~2-15min TTL | **device pool (the golden goose)** |
| Castle coupon | per token | **6 uses**, ≥300s TTL | token pool, retire at 5 |
| Auth rate limit | per session on /auth/* | trips ~300 mints/burst, ~20min TTL | per-account JWT cache |
