# Gateway validation audit — 2026-10-09

Systematic battery probes against `api.framer.com` inference routes from the
`evasion` account (user ce488723, team 8523b5a9, project LzyGImOXpOj63igRbwzW),
in-page Castle mint, ~70 requests total. Ledger pinned at 500 / 0 consumed /
last grant 2026-10-06 throughout — no probe debited anything.

## TL;DR — the gateway does ZERO validation of the inner `request` object

Everything inside the standard envelope's `request` field is a byte-for-byte
tunnel to OpenRouter. Framer never inspects it. Only two envelope fields are
validated: `projectId` (ownership regex) and `featureCategory` (strict
allowlist). The metering decision is keyed to **route + stream flag +
featureCategory** and nothing else.

## Validation map (all on `/ai/v3/chat/`, `stream:false`, unmetered)

Inner `request` fields — ALL passed through unsanitized:

| Probe | Result |
|---|---|
| `max_tokens: -1` | 200 served |
| `max_tokens: 1e18` | 200 served |
| `max_tokens: null` → | 400 OpenRouter zod (leaks `org_2vB2OvmhsdWsL7zCsc0vlP8tCAX`) |
| `model: null` | 400 Framer "model is required in request" (the ONE inner field Framer reads) |
| `model: ["x"]` | 400 OpenRouter "No models provided" |
| `n: 64` / `n: 1000` | 200 served — see amplification note below |
| `logprobs: true, top_logprobs: 20` | 200, full logprobs returned |
| 32 parallel tool schemas | 200 |
| `seed: -2^63` | 200 |
| `stop: null` | 200 |
| `frequency_penalty: -100` | 400 OpenRouter-side only |
| `temperature: "NaN"` | 400 OpenRouter-side only |
| `__proto__` / `constructor` keys | 200 ignored |
| nested `billing:{meter:false,skipCredits:true,free:true}` | 200 ignored |

Content-type: `text/plain` with JSON body → **200** (content-type not
enforced). `application/x-www-form-urlencoded` / `application/xml` → 400
"cannot parse body" (single JSON parser, but doesn't gate on the header).

Envelope fields:

| Probe | Result |
|---|---|
| `stepIndex: -1` | 200 |
| `stepIndex: 2^53` | 200 |
| `step: nudge/suggest/translate/""/null/missing` | 200 (free-form, ignored) |
| `featureName: Triggers/""/null/missing` | 200 (free-form, ignored) |
| `featureCategory: "" / null / internal / missing` | 400 "required and must be valid" |
| `featureCategory: canvas` | 400 "model is not allowed for this feature" (per-category model allowlist) |
| `featureCategory: translation` | 200 served |

## n-amplification: measured no-op

`n:1000` vs `n:1` on gpt-4.1-mini: identical `choices` length (1), identical
`usage.cost` ($6.8e-06), ledger 500 → 500 with no debit at +180s. The provider
behind OpenRouter silently caps `n` to 1; no layer enforces it, no layer
bills it. Not a cost-desync.

## Route surface (existence/gates)

| Route | Gate |
|---|---|
| `/ai/v3/openai/images/generations/` | 403 bare "Forbidden" (feature-gated) |
| `/ai/v3/responses/` (bare) | OpenRouter passthrough — reached upstream, 404 guardrail "0 endpoints… available matching your guardrail restrictions" |
| `/ai/v3/baseten/` | 403 "only available for Framer employees" |
| `/ai/v3/fireworks/` | 403 "only available for Framer employees" |
| `/ai/v3/usage/total/` | GET, needs auth cookie only; param injection battery (traversal/null-byte/array/10k/unicode/dup) → uniform 401 on dead session, retest pending |

## Security observations (org-level)

1. **Org id leakage**: OpenRouter error bodies bubble up unmodified,
   including `user_id: org_2vB2OvmhsdWsL7zCsc0vlP8tCAX` and guardrail
   workspace URLs. Framer makes no attempt to sanitize upstream errors.
2. **Identifier leakage on authz failure**: every 403 includes
   "Access information for project {id} and user {id}" — enumerable oracle.
3. **Raw passthrough = upstream attack surface**: any OpenRouter/provider
   parsing bug is directly reachable through Framer with a valid session.
   Framer adds no hardening.
4. **Refund path**: client posts `{turnId, projectId, sessionId,
   featureCategory}` — server-side binding of turnId→charge holds (phantom
   refunds denied 2026-10-08), quota is effectively one-time per account.

## What was ruled out today (don't retest)

- Inner-request numeric overflow / negative values — passthrough, harmless.
- Content-type confusion on chat route — header ignored, JSON required.
- Prototype pollution keys — ignored.
- Envelope step/featureName/stepIndex manipulation — no metering effect.
- featureCategory escape — strict allowlist.
- n-amplification — provider no-op.
