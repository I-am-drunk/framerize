# Bounded browser and native comparison source

Status: source-only, fixtures UNRUN. Parent: `d0954c6`.
The runtime owner owns integration, validation, account admission and execution.
Neither service was queried or changed by this work.

The observed scope has three explicit commands. Ordinary one-shot authorization
keeps its existing wrapper and behavior. No credential source implicitly sends
the baseline request.

1. `prepare_inference_authorization`, purpose `baseline_context`, binds `project`,
   `path`, `cookie_header`, `body`, `native_body`, `wrapper_url`, `wrapper_export`.
   Both envelopes must be identical except their distinct, nonempty top-level
   `turnId` strings, bounded to 256 UTF-8 bytes. The owner supplies both IDs.
   It seeds only the supplied login session, navigates once and observes the
   actual device in local storage and the current cookie jar. It mints no token.
2. `browser_baseline_once` takes that opaque `context_handle`, `project`, `path`,
   the bound baseline `body`, and private `bearer` plus its exact `cookie_header`.
   It freshly mints and sends one nonstreaming browser request. Its result is
   bounded evidence, including status and refusal policy, not response text.
3. `prepare_inference_authorization`, purpose `native_token`, takes the same
   handle, project and path with the separately bound native `body`. Only a
   successful baseline permits this fresh mint. The private result retains the
   ordinary token/current-cookie shape for the owner's corrected bearer source.

The initial wrapper must explicitly select
`https://app.framerstatic.com/chunk-VPFQZMNG.mjs`, export `b`.
The implementation never silently changes ordinary authorization to that export.

Both bodies require `request.stream=false` and the exact project. Neither body
may supply `castleToken`. The 64 KiB limit applies separately to request and
response JSON. Browser fetch uses the saved public caller's `same-origin`
credential mode, disables redirects and has a 30-second deadline. One scope
retains the same account, generation, browser, context, page, observed device,
project, route and 180-second continuation deadline. Continuations cannot
navigate, reseed, retry, adopt another profile or recreate an absent account.

Baseline evidence contains `source`, `context_handle`, `project`, `path`,
`accepted`, `status`, `content_type`, `response_complete`, `complete_json`,
`nonempty_text`, `body_bytes`, `transport_starts`, `transport_finishes`,
`fetch_settled`, `failure` and `refusal`. Content types are fixed categories.
Refusal is null or a bounded `{policy, basis}` observation using existing 403
policy. That policy is not a finding about token validity or the remote cause.
The counters describe this owned fetch invocation and settlement, not an
independent wire capture or upstream generation. Unknown counts remain null.

HTTP 200 alone cannot pass: complete bounded JSON, the route's completed-result
shape and nonempty text are required. A refusal, malformed body or uncertain
fetch prevents native continuation. Cancellation leaves observer-owned RPCs
joined by the existing scope cleanup. Fetch settlement is not browser cleanup;
the owner retains the original scope through native completion and teardown.

Replies can be visible before their writer-owning commands retire. The next
command joins its exact predecessor before doing any browser work. Existing
command limits remain; allow two command slots for that bookkeeping overlap.

Focused inert test entry: `reference/fixtures/browser_comparison_tests.py`.
It defines 28 cases, including two reply-ordering and predecessor-failure cases.
It covers sequence and replay, separate turn IDs, drift at both barriers,
refusal and completion, bounded private evidence, cancellation and real
observer phase promotion. It loads only inert fixtures and fake page/RPC ports.
These new tests and all candidate Python/JavaScript remain UNRUN here.
