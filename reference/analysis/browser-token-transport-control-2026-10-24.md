# Browser token and native transport comparison

Source assessment at `be7c956a2d503e793d9d1ec24835a72396799e75`.
No SDK, browser, private-state or live request execution accompanied it.

Implementation update: the contextual source now acquires its bearer after
validating the returned browser jar, using those cookies. Token age begins
before the bearer wait. The five contextual cases and forty retained request,
authentication and cleanup cases pass in the focused build. The original
assessment below describes the baseline; source wiring and the matched live
comparison remain unfinished. See `browser-context-validation-20261024.json`.

Castle causation remains unproven. The searched owner and reviewer records
contain no authenticated browser-minted-token to native-HTTP control.
Synthetic adapter checks and discarded tokens do not supply that evidence.

The native sender uses `Framerize/0.1`, HTTP/1, explicit Origin and Referer,
and no inference Cookie header. There is no later User-Agent override in
`experimental_native/native_http.rs`. Its identity agrees with the native
bearer transport and worker. This is not a newly discovered internal mismatch.
The browser uses its own network stack. Its custom fetch shim includes
credentials, unlike the current upstream application's inference caller.
Neither distinction alone identifies the reason for the security refusal.

The existing browser control is incomplete as a matched live experiment:

- `offline_http::configure` selects `OfflineSource`; `Provider::with_sources`
  is an injection point, not an exposed browser-control launch option.
- `main::build_state` creates an unmanaged sidecar. `observer_factory`
  requires the separate observed mode and rejects that unmanaged sidecar.
- `new_observed_one_shot` creates a fresh profile; contextual initialization
  creates a new browser context. It does not adopt the working browser.
- `contextual_authorization.py` seeds only the session cookie. It does not
  establish the same saved device or browser storage as the native trial.
- Its direct call uses export `a` of the host's older wrapper. A bounded,
  nonempty result does not attest to the selected V3 branch.
- `contextual_control` obtains a native bearer from the original session
  before browser preparation. That is not a bearer obtained from the
  browser's resulting cookie jar.
- The observed factory couples egress and retains cleanup ownership;
  constructing `BoundBrowserSessionSource` alone does not provide both.

The next implementation is an explicit isolated comparison using the existing
account, request and cleanup owners. It must first establish a working real
browser baseline for the selected account and endpoint, then use a separate
fresh browser token with native HTTP. Keep the account, legitimate device
state, egress, bearer context, model and request semantics matched. Each leg
needs its own token and turn; never replay a used token or reuse a consumed
request. Do not change the HTTP fingerprint to impersonate a browser.

A successful native leg would show that native HTTP can deliver that matched
request with browser preparation. It would narrow the difference toward
native preparation without identifying a particular SDK defect. If the
browser baseline succeeds and the native leg fails, investigate transport
and request-context differences. If the browser baseline fails, stop without
the native leg: it cannot isolate the earlier failure. Account policy and
backend decisions remain possible. Tokens from the protected service are
not needed.

The unfinished work is source wiring and matched-context fixtures, followed
by one bounded, owned comparison. A new browser consumes additional memory;
its launch must fit the existing protected-service reserve. Preserve actual
cooldowns, reject overlapping ownership, stop on refusal and await cleanup.
Do not turn an account lease into proof of global exclusion, substitute a
permissive `ExclusiveWindow`, or start another unmanaged daemon.

This is source preparation only. Staged binary `2a0de595` is still undeployed;
the last observed isolated binary/configuration remains `3e9398d8/693051b1`.
No new current-process identity or authenticated success is claimed.
