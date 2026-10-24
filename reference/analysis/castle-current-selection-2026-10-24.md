# Public Castle selection checked on 2026-10-24

Unauthenticated public assets fetched at 01:12:41–01:13:48 UTC show that
Framer has changed its wrapper, while retaining the same V3 SDK and
publishable key as the frozen native worker.

The [login page](https://framer.com/login/) declared stable build
`45ee59fa9c68253136b8a3322d3c4dc160671fbb`, login entry
`login.ESAUFACN.mjs`, and editor prefetch entry `web.S5TGFD7C.mjs`.
The login entry's `useCastle` hook (`chunk-4ZRCCDU5.mjs`) exposes the new
wrapper's export `b` as `createToken`. The editor entry imports its
initialization export `a`. This is static selection evidence, not observed
authenticated editor execution. An unauthenticated projects GET redirected
to the same login page.

| Public asset at `https://app.framerstatic.com/` | SHA256 |
| --- | --- |
| Current wrapper `chunk-VPFQZMNG.mjs` | `c32148dcc8131bd6fb7a5c15bc7b183e8dc38f9941779caab1e6f1015b8255c8` |
| Frozen wrapper `chunk-5LKSDTQ5.mjs`, still available | `cf52d0c8822a441b149fbee55f6832eec006ee1c63cf5f8538159437fb072ef0` |
| V3 `castleSDKV3-LNGA36I3.mjs`, selected by both | `3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71` |

Both wrappers configure V3 with `pk` and `onError`. The current wrapper
uses a ten-second default V3 budget, conditional early initialization,
source-labelled diagnostics and token timing telemetry. The frozen wrapper
uses five seconds. The current wrapper has six static dependencies instead
of three; `a` now initializes and `b` generates tokens. Replacing only the
worker's file pins would therefore call the wrong export.

The public key matches the saved graphics-worker source. Its value is
omitted. Local comparison `18ca8216c8f898edaf532c65f8532fce2d9749e56bd0accce6e67daa8ae1a90d`
binds ten fetched objects, their headers/hashes, and exact source excerpts.
Focused REA login/hook analysis was partial; SDK internals were not rerun.

These findings establish wrapper drift, not the cause of the security 403.
Effective account flags remain unobserved. No SDK, inference, deployment or
service action occurred. A current-wrapper candidate needs its own dependency,
export, lifecycle and resource qualification. Graphics cancellation remains
an independent unfinished check. Existing accepted results remain valid for
their frozen inputs; they do not validate the new wrapper.
