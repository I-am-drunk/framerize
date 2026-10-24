# Public AI request contract, 2026-10-24

These are saved public-source findings, not explanations of R16's HTTP 403.
R16's browser baseline received a complete security refusal. It did not send
the native leg, so it does not isolate native transport or offline SDK behavior.

In `chunk-5PNX2WXY.mjs`, `zD` at bytes [296098,296351) reconstructs Authorization
and Content-Type, with optional Anthropic beta and credits-experiment headers.
Its Content-Type fallback is exactly `application/json`; incoming SDK headers
can override it. It does not add `X-Requested-By`. The selected application
constructor in `chunk-OEY74USB.mjs`, `LM` at [990142,990312), supplies baseURL,
fetch and apiKey, without custom headers.

Both public Responses paths remove `model`, `reasoningEffort` and
`dedicatedCompaction` from outer metadata. The model remains inside `request`.
This covers `/ai/v3/responses/` and `/ai/v3/openai/responses/`, and does not
establish the Messages contract. The native correction preserves that boundary.

The app checks its workspace AI-disabled policy before bearer acquisition and
minting. The exact predicate is shared export `c(engine)`. Engine export `a()`
from `chunk-WC34TENX.mjs` reads a mutable pointer, so a usable observation must
match the bound project and actual space membership. Missing context is unknown.

The wrapper's existing `chunk-WBI6GGAG.mjs` dependency exposes its logger buffer.
Records have increasing numeric IDs and a logger object. In the same realm,
`i('castle')` returns the wrapper's logger object. Fixed counts of new records
can expose V3/V2 selection and allowlisted failures. Eviction limits coverage;
concurrent page calls prevent assigning every record to one returned token.

The comparison retains bounded reads, `redirect: 'error'`, and `cache: 'no-store'`.
The public caller leaves those fetch defaults. R16's complete HTTP 403 was not
a local redirect refusal. No redirect or cleanup guard was relaxed.

Saved source SHA-256 bindings:

```text
5PNX2WXY e909f8a59337a4f0d63c92edccf7d25f6660b80b9ffcbb87bb612bf7db99263f
OEY74USB 5f6b435dfbb92e11d6f6c48e1a1abb4cde00dc7e6d8c9f7b48af8649b844b19d
WC34TENX 05377d4f857d937448240dc6e6c96c596373977b49a81747bbb857fec99482c1
WBI6GGAG c04efb9e664789b5b0085eabebcf35e47c9a93d124ab9622b174bd9722e18046
```
