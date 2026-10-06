# T3 ultrafast model variants

**Feature**: model ids ending in `-ultrafast` request OpenRouter's
`service_tier: "ultrafast"` upstream (OpenAI's lowest-latency tier — the
docs' own example is `gpt-6-astra` + ultrafast).

## How it works (one hook, deliberately tiny)

`crates/framer-server/src/lib.rs` `free_complete()` intercepts any model id
ending in `-ultrafast` **before routing**:

1. strips the suffix (`gpt-6-astra-ultrafast` → `gpt-6-astra`),
2. injects `"service_tier": "ultrafast"` into the payload.

Everything downstream is unchanged:

- **Free-route models** (astra): conversions carry `service_tier` to the
  OpenRouter passthrough — **still unmetered** (verified: tier=ultrafast
  served, ledger delta 0.0).
- **Direct-route models** (sol): compat_forward sends the payload verbatim,
  so the tier rides to the direct route (metered, credit-bond floor applies).

## T3 side

Two slugs in `~/.t3/userdata/settings.json` → `providers.codex.customModels`:

```json
{ "slug": "gpt-6-astra-ultrafast", "name": "GPT-6 Astra Ultrafast" },
{ "slug": "gpt-6.1-sol-ultrafast", "name": "GPT-6.1 Sol Ultrafast" }
```

## Verified

- `/openai/v1/responses` `gpt-6-astra-ultrafast` → 200, `service_tier:
  "ultrafast"` echoed, ledger unmoved.
- `gpt-6.1-sol-ultrafast` → 200 on the direct route, tier echoed (metered).
