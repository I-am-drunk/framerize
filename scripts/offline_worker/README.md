This source adds explicit saved-device storage to the isolated worker.

Version 2 requires one supplied device value. The worker seeds localStorage
and the __cuid cookie before evaluating the original SDK wrapper, then checks
both values before closing the window. Missing, malformed or changed values
fail the operation. The device handle rejects JSON serialization, and public
metadata records only seeded/unchanged booleans. Version 1 retains its original
stateless behavior and account_state_loaded remains false in both versions.

The SDK, wrapper, branch observation, token writer and process cleanup stay
unchanged. readInput now returns parsed input for the constructor's single
validation, preserving the private handle through its lifetime.

The 19 existing inert cases remain. Nine additional cases cover explicit input,
privacy, real JSDOM storage, separate windows, repeated saved values and rejected
storage changes. No case evaluates the SDK or contacts a service.

Status: source only, untested. This component accepts a value; the application
must obtain it from the selected account's existing authority and rate ledger.
It does not establish token-internal identity, upstream acceptance, account
continuity, live integration or deployment. All earlier freezes are preserved.
