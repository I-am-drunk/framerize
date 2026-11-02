# Refusals after a divergent release replaced the verified build

The service journal records alpha.12, source `38f3c583`, at 02:43:24 UTC.
At 03:10:00 it changed to alpha.1, source `d578e93e`, from the separate 0.6.0
development line. That source does not contain the preceding browser repairs.
The diagnosis and verification in this record have not restarted production.

The replacement's typed retry policy treats a security refusal lasting more
than eight seconds as a transport failure. Production logged those retries
and global refusal pauses of 150, 300, 600 and 900 seconds. The global pause
is protective. Retrying a known security refusal is the defect; elapsed time
does not establish a different cause. Framer did not disclose the private
reason for its original security decision.

The service later returned to the Camoufox engine while retaining `d578e93e`.
That combination reintroduced cached-part token generation, and its log
contains `castle.forge` events. It also omits the previously verified browser
retention and local timing repairs. The candidate is the exact alpha.12
browser package, which obtains fresh tokens from the account's browser SDK.
No changes to the separate native experiments are part of this repair.

The replacement's apply script verified integrity and HTTP health but did
not require successful inference before stopping production. The supported
activation tool now requires a recent receipt tied to the exact binary and
package manifest. Completed buffered and streaming requests must pass on
both OpenAI Responses and Claude Messages. Credit errors, incomplete HTTP
200 responses, shared accounts, metered routes and changed evidence fail
before any stop or pointer change. Interrupted-update recovery remains
available without trying the failed candidate again.

Read-only session checks found a signed-in identity and an existing workspace
outside all twelve production accounts and all 28 workspace assignments. That
workspace reports 500 credits and has existing projects. Verification uses
one pinned account and project, with separate state, profiles and logs.
The previous test workspace remains exhausted. No credits were purchased,
refunded or manufactured, and no replacement workspace was created.

The recovered account also exposed two setup defects. Credit scans treated
personal spaces as teams, causing HTTP 403 and incomplete workspace discovery.
Scans and rotation now select only organization spaces. Browser configuration
now finishes before marking a browser launch in the durable journal. Missing
installation metadata cannot leave an account behind a false cleanup fence.
An actual unconfirmed launch still blocks admission and retains its owner.

Docker could not establish the systemd proof needed after the initial failed
launch. The stopped test container's complete account history was transferred
to the local isolated service. Existing cross-boot recovery preserved it and
applied the full 600-second cooldown. No history was erased or manually cleared.

Status: 68 focused Python checks pass. Alpha.13 packaging and real-request
verification remain required. Production must not be replaced before they pass.
